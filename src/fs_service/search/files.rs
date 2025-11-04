use crate::{
    error::ServiceResult,
    fs_service::{FileSystemService, utils::filesize_in_range},
};
use glob_match::glob_match;
use ignore::WalkBuilder;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path};
use tokio::{fs::File, io::AsyncReadExt};

impl FileSystemService {
    /// Searches for files in the directory tree starting at `root_path` that match the given `pattern`,
    /// excluding paths that match any of the `exclude_patterns`.
    ///
    /// # Arguments
    /// * `root_path` - The root directory to start the search from.
    /// * `pattern` - A glob pattern to match file names (case-insensitive). If no wildcards are provided,
    ///   the pattern is wrapped in '*' for partial matching.
    /// * `exclude_patterns` - A list of glob patterns to exclude paths (case-sensitive).
    ///
    /// # Returns
    /// A `ServiceResult` containing a vector of`walkdir::DirEntry` objects for matching files,
    /// or a `ServiceError` if an error occurs.
    pub async fn search_files(
        &self,
        root_path: &Path,
        pattern: String,
        exclude_patterns: Vec<String>,
        file_extensions: Option<Vec<String>>,
        min_bytes: Option<u64>,
        max_bytes: Option<u64>,
    ) -> ServiceResult<Vec<ignore::DirEntry>> {
        let result = self
            .search_files_iter(root_path, pattern, exclude_patterns, file_extensions, min_bytes, max_bytes)
            .await?;
        Ok(result.collect::<Vec<ignore::DirEntry>>())
    }

    /// Returns an iterator over files in the directory tree starting at `root_path` that match
    /// the given `pattern`, excluding paths that match any of the `exclude_patterns`.
    ///
    /// # Arguments
    /// * `root_path` - The root directory to start the search from.
    /// * `pattern` - A glob pattern to match file names. If no wildcards are provided, the pattern is wrapped in `**/*{pattern}*` for partial matching.
    /// * `exclude_patterns` - A list of glob patterns to exclude paths (case-sensitive).
    ///
    /// # Returns
    /// A `ServiceResult` containing an iterator yielding `walkdir::DirEntry` objects for matching files,
    /// or a `ServiceError` if an error occurs.
    pub async fn search_files_iter<'a>(
        &'a self,
        // root_path: impl Into<PathBuf>,
        root_path: &'a Path,
        pattern: String,
        exclude_patterns: Vec<String>,
        file_extensions: Option<Vec<String>>,
        min_bytes: Option<u64>,
        max_bytes: Option<u64>,
    ) -> ServiceResult<impl Iterator<Item = ignore::DirEntry> + 'a> {
        let allowed_directories = self.allowed_directories().await;
        let valid_path = self.validate_path(root_path, allowed_directories)?;

        let updated_pattern = if pattern.contains('*') {
            pattern.to_lowercase()
        } else {
            format!("**/*{}*", &pattern.to_lowercase())
        };
        let glob_pattern = updated_pattern;

        let valid_path_for_filter = valid_path.clone();

        let result = WalkBuilder::new(valid_path)
            .follow_links(false)  // Disable follow_links to prevent infinite loops
            .max_depth(Some(20))  // Limit maximum depth to prevent excessive traversal
            .git_ignore(true)     // Respect .gitignore files (default: true)
            .git_global(true)     // Respect global gitignore (default: true)
            .git_exclude(true)    // Respect .git/info/exclude (default: true)
            .ignore(true)         // Respect .ignore files (default: true)
            .hidden(true)         // Skip hidden files (default: true)
            .parents(true)        // Read ignore files from parent directories (default: true)
            .build()
            .filter_map(|v| v.ok())
            .filter(move |entry| {
                let path = entry.path();

                // Skip the root directory itself
                if valid_path_for_filter == path {
                    return false;
                }

                // Apply custom exclude patterns if provided
                if !exclude_patterns.is_empty() {
                    let relative_path = path.strip_prefix(&valid_path_for_filter).unwrap_or(path);
                    let should_exclude = exclude_patterns.iter().any(|pattern| {
                        let glob_pattern = if pattern.contains('*') {
                            pattern.strip_prefix("/").unwrap_or(pattern).to_owned()
                        } else {
                            format!("*{pattern}*")
                        };
                        glob_match(&glob_pattern, relative_path.to_str().unwrap_or(""))
                    });
                    if should_exclude {
                        return false;
                    }
                }

                // Check if the name matches the pattern
                if !glob_match(
                    &glob_pattern,
                    &entry.file_name().to_str().unwrap_or("").to_lowercase(),
                ) {
                    return false;
                }

                // Filter by file extensions if specified
                if let Some(ref exts) = file_extensions {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if !exts.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                            return false;
                        }
                    } else {
                        // No extension, exclude if extensions filter is set
                        return false;
                    }
                }

                // Only check file size constraints if specified and entry is a file
                if (min_bytes.is_some() || max_bytes.is_some())
                    && entry.file_type().map_or(false, |ft| ft.is_file())
                {
                    if let Ok(metadata) = entry.metadata() {
                        return filesize_in_range(metadata.len(), min_bytes, max_bytes);
                    }
                    // If we can't get metadata, exclude the file when size filters are set
                    return false;
                }

                true
            });        Ok(result)
    }

    /// Finds groups of duplicate files within the given root path.
    /// Returns a vector of vectors, where each inner vector contains paths to files with identical content.
    /// Files are considered duplicates if they have the same size and SHA-256 hash.
    pub async fn find_duplicate_files(
        &self,
        root_path: &Path,
        pattern: Option<String>,
        exclude_patterns: Option<Vec<String>>,
        min_bytes: Option<u64>,
        max_bytes: Option<u64>,
    ) -> ServiceResult<Vec<Vec<String>>> {
        // Validate root path against allowed directories
        let allowed_directories = self.allowed_directories().await;
        let valid_path = self.validate_path(root_path, allowed_directories)?;

        // Get Tokio runtime handle
        let rt = tokio::runtime::Handle::current();

        // Step 1: Collect files and group by size
        let mut size_map: HashMap<u64, Vec<String>> = HashMap::new();
        let entries = self
            .search_files_iter(
                &valid_path,
                pattern.unwrap_or("**/*".to_string()),
                exclude_patterns.unwrap_or_default(),
                None,  // No file extension filter
                min_bytes,
                max_bytes,
            )
            .await?
            .filter(|e| e.file_type().map_or(false, |ft| ft.is_file())); // Only files

        for entry in entries {
            if let Ok(metadata) = entry.metadata()
                && let Some(path_str) = entry.path().to_str()
            {
                size_map
                    .entry(metadata.len())
                    .or_default()
                    .push(path_str.to_string());
            }
        }

        // Filter out sizes with only one file (no duplicates possible)
        let size_groups: Vec<Vec<String>> = size_map
            .into_iter()
            .collect::<Vec<_>>() // Collect into Vec to enable parallel iteration
            .into_par_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(_, paths)| paths)
            .collect();

        // Step 2: Group by quick hash (first 4KB)
        let mut quick_hash_map: HashMap<Vec<u8>, Vec<String>> = HashMap::new();
        for paths in size_groups.into_iter() {
            let quick_hashes: Vec<(String, Vec<u8>)> = paths
                .into_par_iter()
                .filter_map(|path| {
                    let rt = rt.clone(); // Clone the runtime handle for this task
                    rt.block_on(async {
                        let file = File::open(&path).await.ok()?;
                        let mut reader = tokio::io::BufReader::new(file);
                        let mut buffer = vec![0u8; 4096]; // Read first 4KB
                        let bytes_read = reader.read(&mut buffer).await.ok()?;
                        let mut hasher = Sha256::new();
                        hasher.update(&buffer[..bytes_read]);
                        Some((path, hasher.finalize().to_vec()))
                    })
                })
                .collect();

            for (path, hash) in quick_hashes {
                quick_hash_map.entry(hash).or_default().push(path);
            }
        }

        // Step 3: Group by full hash for groups with multiple files
        let mut full_hash_map: HashMap<Vec<u8>, Vec<String>> = HashMap::new();
        let filtered_quick_hashes: Vec<(Vec<u8>, Vec<String>)> = quick_hash_map
            .into_iter()
            .collect::<Vec<_>>()
            .into_par_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect();

        for (_quick_hash, paths) in filtered_quick_hashes {
            let full_hashes: Vec<(String, Vec<u8>)> = paths
                .into_par_iter()
                .filter_map(|path| {
                    let rt = rt.clone(); // Clone the runtime handle for this task
                    rt.block_on(async {
                        let file = File::open(&path).await.ok()?;
                        let mut reader = tokio::io::BufReader::new(file);
                        let mut hasher = Sha256::new();
                        let mut buffer = vec![0u8; 8192]; // 8KB chunks
                        loop {
                            let bytes_read = reader.read(&mut buffer).await.ok()?;
                            if bytes_read == 0 {
                                break;
                            }
                            hasher.update(&buffer[..bytes_read]);
                        }
                        Some((path, hasher.finalize().to_vec()))
                    })
                })
                .collect();

            for (path, hash) in full_hashes {
                full_hash_map.entry(hash).or_default().push(path);
            }
        }

        // Collect groups of duplicates (only groups with more than one file)
        let duplicates: Vec<Vec<String>> = full_hash_map
            .into_values()
            .filter(|group| group.len() > 1)
            .collect();

        Ok(duplicates)
    }
}
