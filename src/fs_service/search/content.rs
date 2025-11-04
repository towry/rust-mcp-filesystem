use crate::{
    error::ServiceResult,
    fs_service::{FileSystemService, utils::escape_regex},
};
use glob_match::glob_match;
use grep::{
    matcher::{Match, Matcher},
    regex::RegexMatcherBuilder,
    searcher::{BinaryDetection, Searcher, sinks::UTF8},
};
use ignore::WalkBuilder;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const SNIPPET_MAX_LENGTH: usize = 200;
const SNIPPET_BACKWARD_CHARS: usize = 30;

/// Represents a single match found in a file's content.
#[derive(Debug, Clone)]
pub struct ContentMatchResult {
    /// The line number where the match occurred (1-based).
    pub line_number: u64,
    pub start_pos: usize,
    /// The line of text containing the match.
    /// If the line exceeds 255 characters (excluding the search term), only a truncated portion will be shown.
    pub line_text: String,
}

/// Represents all matches found in a specific file.
#[derive(Debug, Clone)]
pub struct FileSearchResult {
    /// The path to the file where matches were found.
    pub file_path: PathBuf,
    /// All individual match results within the file.
    pub matches: Vec<ContentMatchResult>,
}

impl FileSystemService {
    // Searches the content of a file for occurrences of the given query string.
    ///
    /// This method searches the file specified by `file_path` for lines matching the `query`.
    /// The search can be performed as a regular expression or as a literal string,
    /// depending on the `is_regex` flag.
    ///
    /// If matched line is larger than 255 characters, a snippet will be extracted around the matched text.
    ///
    pub fn content_search(
        &self,
        query: &str,
        file_path: impl AsRef<Path>,
        is_regex: Option<bool>,
    ) -> ServiceResult<Option<FileSearchResult>> {
        let query = if is_regex.unwrap_or_default() {
            query.to_string()
        } else {
            escape_regex(query)
        };

        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(true)
            .build(query.as_str())?;

        let mut searcher = Searcher::new();
        let mut result = FileSearchResult {
            file_path: file_path.as_ref().to_path_buf(),
            matches: vec![],
        };

        searcher.set_binary_detection(BinaryDetection::quit(b'\x00'));

        searcher.search_path(
            &matcher,
            file_path,
            UTF8(|line_number, line| {
                let actual_match = matcher.find(line.as_bytes())?.unwrap();

                result.matches.push(ContentMatchResult {
                    line_number,
                    start_pos: actual_match.start(),
                    line_text: self.extract_snippet(line, actual_match, None, None),
                });
                Ok(true)
            }),
        )?;

        if result.matches.is_empty() {
            return Ok(None);
        }

        Ok(Some(result))
    }

    /// Extracts a snippet from a given line of text around a match.
    ///
    /// Static helper function that doesn't depend on self, enabling use in parallel contexts.
    fn extract_snippet_static(
        line: &str,
        match_result: Match,
        max_length: usize,
        backward_chars: usize,
    ) -> String {
        // Calculate the number of leading whitespace bytes to adjust for trimmed input
        let start_pos = line.len() - line.trim_start().len();
        // Trim leading and trailing whitespace from the input line
        let line = line.trim();

        // Calculate the desired start byte index by adjusting match start for trimming and backward chars
        let desired_start = (match_result.start() - start_pos).saturating_sub(backward_chars);

        // Find the nearest valid UTF-8 character boundary at or after desired_start
        let snippet_start = line
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= desired_start)
            .unwrap_or(desired_start.min(line.len()));

        let mut char_count = 0;

        // Calculate the desired end byte index by counting max_length characters from snippet_start
        let desired_end = line[snippet_start..]
            .char_indices()
            .take(max_length + 1)
            .find(|&(_, _)| {
                char_count += 1;
                char_count > max_length
            })
            .map(|(i, _)| snippet_start + i)
            .unwrap_or(line.len());

        // Ensure snippet_end is a valid UTF-8 character boundary at or after desired_end
        let snippet_end = line
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= desired_end)
            .unwrap_or(line.len())
            .min(line.len());

        // Extract the snippet from the trimmed line using the calculated byte indices
        let snippet = &line[snippet_start..snippet_end];

        let mut result = String::new();
        // Add leading ellipsis if the snippet doesn't start at the beginning of the trimmed line
        if snippet_start > 0 {
            result.push_str("...");
        }

        result.push_str(snippet);

        // Add trailing ellipsis if the snippet doesn't reach the end of the trimmed line
        if snippet_end < line.len() {
            result.push_str("...");
        }
        result
    }

    /// Extracts a snippet from a given line of text around a match.
    ///
    /// Instance method wrapper that calls the static helper.
    pub fn extract_snippet(
        &self,
        line: &str,
        match_result: Match,
        max_length: Option<usize>,
        backward_chars: Option<usize>,
    ) -> String {
        Self::extract_snippet_static(
            line,
            match_result,
            max_length.unwrap_or(SNIPPET_MAX_LENGTH),
            backward_chars.unwrap_or(SNIPPET_BACKWARD_CHARS),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn search_files_content(
        &self,
        root_path: impl AsRef<Path>,
        pattern: &str,
        query: &str,
        is_regex: bool,
        exclude_patterns: Option<Vec<String>>,
        min_bytes: Option<u64>,
        max_bytes: Option<u64>,
    ) -> ServiceResult<Vec<FileSearchResult>> {
        let root_path = root_path.as_ref();

        // Validate root path once
        self.validate_path(root_path, self.allowed_directories().await)?;

        // Build parallel walker with ignore crate
        let mut builder = WalkBuilder::new(root_path);
        builder
            .follow_links(false)
            .max_depth(Some(20))
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true);

        // Shared results vector protected by mutex
        let results = Arc::new(Mutex::new(Vec::new()));
        let results_clone = Arc::clone(&results);

        // Clone data for the parallel closure
        let file_pattern = pattern.to_string();
        let search_query = if is_regex {
            query.to_string()
        } else {
            escape_regex(query)
        };
        let exclude_patterns_clone = exclude_patterns.clone();

        // Use build_parallel for concurrent directory traversal + content search
        builder.build_parallel().run(|| {
            let results = Arc::clone(&results_clone);
            let file_pattern = file_pattern.clone();
            let search_query = search_query.clone();
            let exclude_patterns = exclude_patterns_clone.clone();

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

                // Apply file pattern filter
                if !glob_match(&file_pattern, path.to_string_lossy().as_ref()) {
                    return WalkState::Continue;
                }

                // Apply exclude patterns
                if let Some(ref excludes) = exclude_patterns {
                    let path_str = path.to_string_lossy();
                    if excludes.iter().any(|pattern| glob_match(pattern, &path_str)) {
                        return WalkState::Continue;
                    }
                }

                // Apply file size filters
                if min_bytes.is_some() || max_bytes.is_some() {
                    if let Ok(metadata) = entry.metadata() {
                        let size = metadata.len();
                        if let Some(min) = min_bytes {
                            if size < min {
                                return WalkState::Continue;
                            }
                        }
                        if let Some(max) = max_bytes {
                            if size > max {
                                return WalkState::Continue;
                            }
                        }
                    }
                }

                // Perform content search on this file
                if let Ok(file_result) = Self::search_file_content_static(
                    &search_query,
                    path,
                ) {
                    if let Some(file_result) = file_result {
                        let mut results = results.lock().unwrap();
                        results.push(file_result);
                    }
                }

                WalkState::Continue
            })
        });

        // Extract results from mutex
        let results = match Arc::try_unwrap(results) {
            Ok(mutex) => mutex.into_inner().unwrap(),
            Err(arc) => arc.lock().unwrap().clone(),
        };

        Ok(results)
    }

    /// Static helper method for searching file content (used in parallel walker).
    /// Does not depend on self, enabling use in parallel closures.
    fn search_file_content_static(
        query: &str,
        file_path: &Path,
    ) -> ServiceResult<Option<FileSearchResult>> {
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(true)
            .build(query)?;

        let mut searcher = Searcher::new();
        searcher.set_binary_detection(BinaryDetection::quit(b'\x00'));

        let mut matches = Vec::new();
        let matcher_ref = &matcher;

        searcher.search_path(
            matcher_ref,
            file_path,
            UTF8(|line_number, line| {
                if let Ok(Some(m)) = matcher_ref.find(line.as_bytes()) {
                    let start_pos = m.start();
                    let line_text = Self::extract_snippet_static(
                        line,
                        m,
                        SNIPPET_MAX_LENGTH,
                        SNIPPET_BACKWARD_CHARS,
                    );

                    matches.push(ContentMatchResult {
                        line_number,
                        start_pos,
                        line_text,
                    });
                }
                Ok(true)
            }),
        )?;

        if matches.is_empty() {
            return Ok(None);
        }

        Ok(Some(FileSearchResult {
            file_path: file_path.to_path_buf(),
            matches,
        }))
    }
}
