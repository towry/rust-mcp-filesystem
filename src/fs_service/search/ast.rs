use crate::{
    error::{ServiceError, ServiceResult},
    fs_service::FileSystemService,
};
use ast_grep_core::Pattern;
use ast_grep_language::{SupportLang, LanguageExt};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
        Arc,
    },
};

/// Represents a single AST match found in a file.
#[derive(Debug, Clone)]
pub struct AstMatchResult {
    /// The matched code snippet
    pub matched_code: String,
    /// The line number where the match starts (1-based)
    pub line_number: usize,
    /// The column number where the match starts (1-based)
    pub column: usize,
    /// The byte range of the match
    pub byte_range: (usize, usize),
}

/// Represents all AST matches found in a specific file.
#[derive(Debug, Clone)]
pub struct AstFileSearchResult {
    /// The path to the file where matches were found
    pub file_path: PathBuf,
    /// All individual AST match results within the file
    pub matches: Vec<AstMatchResult>,
}

impl FileSystemService {
    /// Searches code using AST pattern matching.
    ///
    /// This method uses ast-grep to perform structural code search.
    /// The pattern is written like ordinary code (e.g., `function $NAME($ARGS) { $BODY }`)
    /// and uses `$UPPERCASE` as wildcards to match any AST node.
    ///
    /// # Arguments
    /// * `pattern` - The AST pattern to search for (e.g., "const $VAR = $VALUE")
    /// * `file_path` - The file to search in
    /// * `language` - The programming language (e.g., "typescript", "rust", "python")
    ///
    /// # Example patterns
    /// - `function $NAME() {}` - Find all no-argument functions
    /// - `if ($COND) { $BODY }` - Find all if statements
    /// - `import { $ITEMS } from '$MODULE'` - Find all named imports
    pub fn ast_search(
        &self,
        pattern: &str,
        file_path: impl AsRef<Path>,
        language: &str,
    ) -> ServiceResult<Option<AstFileSearchResult>> {
        let file_path = file_path.as_ref();

        // Parse the language
        let lang = self.parse_language(language)?;

        // Validate the pattern by trying to parse it
        self.validate_pattern(pattern, lang)?;

        // Read file content
        let content = std::fs::read_to_string(file_path)?;

        // Parse the code into AST using ast-grep
        let root = lang.ast_grep(&content);

        // Create pattern matcher
        let pattern = Pattern::new(pattern, lang);

        // Find all matches
        let matches: Vec<_> = root.root()
            .find_all(pattern)
            .map(|node_match| {
                let node = node_match.get_node();
                let range = node.range();
                let start_pos = node.start_pos();

                AstMatchResult {
                    matched_code: node.text().to_string(),
                    line_number: start_pos.line() + 1, // Convert to 1-based
                    column: start_pos.column(&node) + 1, // Convert to 1-based
                    byte_range: (range.start, range.end),
                }
            })
            .collect();

        if matches.is_empty() {
            return Ok(None);
        }

        Ok(Some(AstFileSearchResult {
            file_path: file_path.to_path_buf(),
            matches,
        }))
    }

    /// Validates an AST pattern by attempting to parse it.
    /// Returns an error if the pattern is invalid.
    fn validate_pattern(&self, pattern: &str, lang: SupportLang) -> ServiceResult<()> {
        use crate::error::ServiceError;

        // Try to parse the pattern as code
        let root = lang.ast_grep(pattern);

        // Check if the pattern parsed to an empty AST (complete failure)
        let root_text = root.root().text();
        if root_text.is_empty() && !pattern.trim().is_empty() {
            return Err(ServiceError::FromString(format!(
                "Invalid AST pattern syntax. The pattern could not be parsed as valid {} code.\n\
                Please check the pattern syntax and ensure it follows the language grammar.\n\
                Documentation: https://ast-grep.github.io/guide/pattern-syntax.html",
                lang.to_string()
            )));
        }

        // Check if the parsed AST contains ERROR nodes, which indicates syntax errors
        let pattern_ast = Pattern::new(pattern, lang);
        if pattern_ast.has_error() {
            return Err(ServiceError::FromString(format!(
                "Invalid AST pattern syntax. The pattern contains syntax errors (ERROR nodes).\n\
                Please verify the pattern follows valid {} syntax.\n\
                Documentation: https://ast-grep.github.io/guide/pattern-syntax.html",
                lang.to_string()
            )));
        }

        Ok(())
    }    /// Parse language string to ast-grep Language
    fn parse_language(&self, language: &str) -> ServiceResult<SupportLang> {
        use crate::error::ServiceError;
        let lang = match language.to_lowercase().as_str() {
            "typescript" | "ts" => SupportLang::TypeScript,
            "tsx" => SupportLang::Tsx,
            "javascript" | "js" => SupportLang::JavaScript,
            "python" | "py" => SupportLang::Python,
            "rust" | "rs" => SupportLang::Rust,
            "go" => SupportLang::Go,
            "java" => SupportLang::Java,
            "kotlin" | "kt" => SupportLang::Kotlin,
            "cpp" | "c++" | "cxx" => SupportLang::Cpp,
            "c" => SupportLang::C,
            "csharp" | "c#" | "cs" => SupportLang::CSharp,
            "swift" => SupportLang::Swift,
            "ruby" | "rb" => SupportLang::Ruby,
            "php" => SupportLang::Php,
            "html" => SupportLang::Html,
            "css" => SupportLang::Css,
            "json" => SupportLang::Json,
            "yaml" | "yml" => SupportLang::Yaml,
            "bash" | "sh" => SupportLang::Bash,
            "lua" => SupportLang::Lua,
            "elixir" | "ex" => SupportLang::Elixir,
            "scala" => SupportLang::Scala,
            "haskell" | "hs" => SupportLang::Haskell,
            "solidity" | "sol" => SupportLang::Solidity,
            "nix" => SupportLang::Nix,
            "hcl" | "terraform" => SupportLang::Hcl,
            _ => return Err(ServiceError::FromString(format!("Unsupported language: {}", language))),
        };
        Ok(lang)
    }

    /// Search for AST patterns across multiple files.
    ///
    /// This method combines file discovery (via ignore crate) with AST pattern matching.
    /// It's useful for finding code patterns across an entire codebase.
    ///
    /// # Performance Considerations
    /// - Uses parallel processing (rayon) for better performance on multi-core systems
    /// - Uses ignore crate's WalkBuilder for efficient file traversal with gitignore support
    /// - Limits maximum files to prevent excessive processing time
    /// - For large codebases, consider narrowing down the search with more specific patterns
    ///
    /// # Arguments
    /// * `root_path` - The directory to search in
    /// * `file_pattern` - Glob pattern for file matching (e.g., "*.ts", "src/**/*.rs")
    /// * `ast_pattern` - The AST pattern to search for
    /// * `language` - The programming language
    /// * `exclude_patterns` - Optional patterns to exclude (applied during file traversal)
    /// * `file_extensions` - Optional file extensions filter (e.g., ["ts", "tsx"])
    pub async fn search_files_ast(
        &self,
        root_path: impl AsRef<Path>,
        file_pattern: &str,
        ast_pattern: &str,
        language: &str,
        exclude_patterns: Option<Vec<String>>,
        file_extensions: Option<Vec<String>>,
    ) -> ServiceResult<Vec<AstFileSearchResult>> {
        const MAX_FILES_WARNING: usize = 2000;
        const MAX_FILES_LIMIT: usize = 10000;
        const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1MB - skip very large files

        let root_path = root_path.as_ref();

        // Parse language and validate pattern upfront before searching files
        let lang = self.parse_language(language)?;
        self.validate_pattern(ast_pattern, lang)?;

        // Validate root path
        self.validate_path(root_path, self.allowed_directories().await)?;

        // Prepare glob filters up-front to avoid recompilation per file
        let include_glob = compile_include_glob(file_pattern)?;
        let include_glob = Arc::new(include_glob);

        let exclude_glob = compile_exclude_glob(exclude_patterns.as_deref())?;
        let exclude_glob = exclude_glob.map(Arc::new);

        let extension_filters = file_extensions
            .as_ref()
            .map(|exts| exts.iter().map(|ext| ext.to_ascii_lowercase()).collect::<Vec<_>>());
        let extension_filters = extension_filters.map(Arc::new);

        // Build walker with ignore crate
        let mut builder = WalkBuilder::new(root_path);
        builder
            .follow_links(false)
            .max_depth(Some(20))
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true)
            .hidden(true);

        // Use channel for result collection (no lock contention)
        let (tx, rx) = mpsc::channel::<AstFileSearchResult>();

        // Use atomic counter (no lock contention)
        let file_count = Arc::new(AtomicUsize::new(0));
        let file_count_clone = Arc::clone(&file_count);

        // Clone data for the parallel closure
        let root_path_buf = root_path.to_path_buf();

        // Create pattern once for reuse
        let pattern_obj = Pattern::new(ast_pattern, lang);
        let pattern_obj = Arc::new(pattern_obj);

        // Use build_parallel for concurrent directory traversal + AST search
        builder.build_parallel().run(|| {
            let tx = tx.clone();
            let file_count = Arc::clone(&file_count_clone);
            let root_path = root_path_buf.clone();
            let pattern_obj = Arc::clone(&pattern_obj);
            let include_glob = Arc::clone(&include_glob);
            let exclude_glob = exclude_glob.clone();
            let extension_filters = extension_filters.clone();

            Box::new(move |entry_result| {
                use ignore::WalkState;

                let entry = match entry_result {
                    Ok(entry) => entry,
                    Err(_) => return WalkState::Continue,
                };

                // Only process files
                let file_type = match entry.file_type() {
                    Some(ft) => ft,
                    None => return WalkState::Continue,
                };

                if !file_type.is_file() {
                    return WalkState::Continue;
                }

                let path = entry.path();

                // Apply file pattern filter - match against relative path for glob patterns
                let relative_path = path.strip_prefix(&root_path).unwrap_or(path);
                if !include_glob.is_match(relative_path) {
                    return WalkState::Continue;
                }

                // Apply exclude patterns to full path
                if let Some(ref excludes) = exclude_glob {
                    if excludes.is_match(path) {
                        return WalkState::Continue;
                    }
                }

                // Apply file extension filter
                if let Some(ref exts) = extension_filters {
                    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                        return WalkState::Continue;
                    };
                    if !exts.iter().any(|allowed| allowed.eq_ignore_ascii_case(ext)) {
                        return WalkState::Continue;
                    }
                }

                // Apply file size filter
                if let Ok(metadata) = entry.metadata() {
                    if metadata.len() > MAX_FILE_SIZE {
                        return WalkState::Continue;
                    }
                }

                // Count only files that pass all filters and will be AST-parsed
                // Use atomic operation (no lock)
                let count = file_count.fetch_add(1, Ordering::Relaxed);
                if count >= MAX_FILES_LIMIT {
                    return WalkState::Quit;
                }

                // Perform AST search on this file
                if let Ok(content) = std::fs::read_to_string(path) {
                    if !content.is_empty() {
                        let root = lang.ast_grep(&content);
                        // Use reference instead of clone (performance fix)
                        let matches: Vec<_> = root.root()
                            .find_all(pattern_obj.as_ref())
                            .map(|node_match| {
                                let node = node_match.get_node();
                                let range = node.range();
                                let start_pos = node.start_pos();

                                AstMatchResult {
                                    matched_code: node.text().to_string(),
                                    line_number: start_pos.line() + 1,
                                    column: start_pos.column(&node) + 1,
                                    byte_range: (range.start, range.end),
                                }
                            })
                            .collect();

                        if !matches.is_empty() {
                            // Send via channel (no lock contention)
                            let _ = tx.send(AstFileSearchResult {
                                file_path: path.to_path_buf(),
                                matches,
                            });
                        }
                    }
                }

                WalkState::Continue
            })
        });

        // Drop sender to close channel
        drop(tx);

        // Collect results from channel
        let results: Vec<AstFileSearchResult> = rx.iter().collect();

        let final_count = file_count.load(Ordering::Relaxed);

        // Provide feedback
        if final_count >= MAX_FILES_LIMIT {
            eprintln!(
                "Warning: AST search hit maximum file limit of {}. Results may be incomplete.",
                MAX_FILES_LIMIT
            );
            eprintln!("Consider narrowing your search with more specific patterns or exclude patterns.");
        } else if final_count >= MAX_FILES_WARNING {
            eprintln!(
                "Info: Searched {} files.",
                final_count
            );
        }

        Ok(results)
    }
}

fn compile_include_glob(pattern: &str) -> ServiceResult<GlobSet> {
    let normalized = if pattern.trim().is_empty() {
        "**/*"
    } else {
        pattern
    };

    let mut builder = GlobSetBuilder::new();
    let glob = Glob::new(normalized).map_err(|err| {
        ServiceError::FromString(format!(
            "Invalid file glob pattern '{normalized}': {err}"
        ))
    })?;
    builder.add(glob);
    builder
        .build()
        .map_err(|err| ServiceError::FromString(format!(
            "Failed to build file glob matcher for pattern '{normalized}': {err}"
        )))
}

fn compile_exclude_glob(patterns: Option<&[String]>) -> ServiceResult<Option<GlobSet>> {
    let Some(patterns) = patterns else {
        return Ok(None);
    };

    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    let mut added = false;

    for pattern in patterns {
        if pattern.trim().is_empty() {
            continue;
        }

        let normalized = if pattern.contains('*') {
            pattern.strip_prefix('/').unwrap_or(pattern).to_owned()
        } else {
            format!("*{pattern}*")
        };

        let glob = Glob::new(&normalized).map_err(|err| {
            ServiceError::FromString(format!(
                "Invalid exclude glob pattern '{pattern}': {err}"
            ))
        })?;
        builder.add(glob);
        added = true;
    }

    if !added {
        return Ok(None);
    }

    builder
        .build()
        .map(Some)
        .map_err(|err| ServiceError::FromString(format!(
            "Failed to build exclude glob patterns: {err}"
        )))
}
