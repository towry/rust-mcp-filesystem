use crate::{
    error::ServiceResult,
    fs_service::FileSystemService,
};
use ast_grep_core::Pattern;
use ast_grep_language::{SupportLang, LanguageExt};
use std::{
    path::{Path, PathBuf},
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

    /// Parse language string to ast-grep Language
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
    /// # Arguments
    /// * `root_path` - The directory to search in
    /// * `file_pattern` - Glob pattern for file matching (e.g., "*.ts", "**/*.rs")
    /// * `ast_pattern` - The AST pattern to search for
    /// * `language` - The programming language
    /// * `exclude_patterns` - Optional patterns to exclude
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
        let files_iter = self
            .search_files_iter(
                root_path.as_ref(),
                file_pattern.to_string(),
                exclude_patterns.unwrap_or_default(),
                file_extensions,
                None, // min_bytes
                None, // max_bytes
            )
            .await?;

        let mut results = Vec::new();

        for entry in files_iter {
            let path = entry.path();

            // Perform AST search on this file
            if let Ok(Some(file_result)) = self.ast_search(ast_pattern, path, language) {
                results.push(file_result);
            }
        }

        Ok(results)
    }
}
