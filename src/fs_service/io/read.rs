use crate::{
    error::ServiceResult,
    fs_service::{
        FileSystemService,
        utils::{
            format_permissions, format_system_time, mime_from_path, read_file_as_base64,
            validate_file_size,
        },
    },
};
use futures::{StreamExt, stream};
use std::fs::{self};
use std::path::Path;
use std::time::SystemTime;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, BufReader},
};

const MAX_CONCURRENT_FILE_READ: usize = 5;

impl FileSystemService {
    pub async fn read_text_file(&self, file_path: &Path) -> ServiceResult<String> {
        let allowed_directories = self.allowed_directories().await;
        let valid_path = self.validate_path(file_path, allowed_directories)?;
        let content = tokio::fs::read_to_string(valid_path).await?;
        Ok(content)
    }

    /// Reads lines from a text file with flexible positioning options, preserving line endings.
    /// Args:
    ///     path: Path to the file
    ///     offset: Number of lines to skip (0-based) from start or end
    ///     limit: Optional maximum number of lines to read
    ///     from_end: If true, reads from the end of the file
    /// Returns a String containing the selected lines with original line endings or an error if the path is invalid or file cannot be read.
    pub async fn read_file_lines(
        &self,
        path: &Path,
        offset: usize,
        limit: Option<usize>,
        from_end: bool,
    ) -> ServiceResult<String> {
        // Validate file path against allowed directories
        let allowed_directories = self.allowed_directories().await;
        let valid_path = self.validate_path(path, allowed_directories)?;

        // Open file and get metadata before moving into BufReader
        let file = File::open(&valid_path).await?;
        let file_size = file.metadata().await?.len();

        // If file is empty or limit is 0, return empty string
        if file_size == 0 || limit == Some(0) {
            return Ok(String::new());
        }

        if from_end {
            // Use rev_lines crate for efficient reverse reading
            let valid_path_clone = valid_path.to_path_buf();
            let result = tokio::task::spawn_blocking(move || -> ServiceResult<String> {
                use std::fs::File;
                use std::io::{BufRead, BufReader};

                // Read file content to detect line ending style
                let file_content = std::fs::read(&valid_path_clone)?;
                let has_crlf = file_content.windows(2).any(|w| w == b"\r\n");
                let line_ending = if has_crlf { "\r\n" } else { "\n" };

                // Read all lines preserving order
                let file = File::open(&valid_path_clone)?;
                let reader = BufReader::new(file);
                let all_lines: Vec<String> = reader.lines().collect::<Result<Vec<_>, _>>()?;

                // Apply offset from end
                if offset >= all_lines.len() {
                    return Ok(String::new());
                }

                // Determine how many lines to read from end
                let lines_to_read = limit
                    .unwrap_or(all_lines.len() - offset)
                    .min(all_lines.len() - offset);

                // Get the slice of lines we need (from end)
                let start_idx = all_lines.len() - offset - lines_to_read;
                let end_idx = all_lines.len() - offset;
                let selected_lines = &all_lines[start_idx..end_idx];

                // Reconstruct the text with proper line endings
                if selected_lines.is_empty() {
                    return Ok(String::new());
                }

                let mut result = selected_lines.join(line_ending);

                // Only add trailing newline if we're reading up to the actual end of file
                if offset == 0 {
                    // Check if original file ends with newline
                    if !file_content.is_empty() && file_content[file_content.len() - 1] == b'\n' {
                        result.push_str(line_ending);
                    }
                }

                Ok(result)
            })
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))??;

            Ok(result)
        } else {
            // Read from start: original logic
            let mut reader = BufReader::new(file);

            // Skip offset lines (0-based indexing)
            let mut buffer = Vec::new();
            for _ in 0..offset {
                buffer.clear();
                if reader.read_until(b'\n', &mut buffer).await? == 0 {
                    return Ok(String::new()); // EOF before offset
                }
            }

            // Read lines up to limit (or all remaining if limit is None)
            let mut result = String::with_capacity(limit.unwrap_or(100) * 100); // Estimate capacity
            match limit {
                Some(max_lines) => {
                    for _ in 0..max_lines {
                        buffer.clear();
                        let bytes_read = reader.read_until(b'\n', &mut buffer).await?;
                        if bytes_read == 0 {
                            break; // Reached EOF
                        }
                        result.push_str(&String::from_utf8_lossy(&buffer));
                    }
                }
                None => {
                    loop {
                        buffer.clear();
                        let bytes_read = reader.read_until(b'\n', &mut buffer).await?;
                        if bytes_read == 0 {
                            break; // Reached EOF
                        }
                        result.push_str(&String::from_utf8_lossy(&buffer));
                    }
                }
            }

            Ok(result)
        }
    }

    pub async fn read_media_files(
        &self,
        paths: Vec<String>,
        max_bytes: Option<usize>,
    ) -> ServiceResult<Vec<(infer::Type, String)>> {
        let results = stream::iter(paths)
            .map(|path| async {
                self.read_media_file(Path::new(&path), max_bytes)
                    .await
                    .map_err(|e| (path, e))
            })
            .buffer_unordered(MAX_CONCURRENT_FILE_READ) // Process up to MAX_CONCURRENT_FILE_READ files concurrently
            .filter_map(|result| async move { result.ok() })
            .collect::<Vec<_>>()
            .await;
        Ok(results)
    }

    pub async fn read_media_file(
        &self,
        file_path: &Path,
        max_bytes: Option<usize>,
    ) -> ServiceResult<(infer::Type, String)> {
        let allowed_directories = self.allowed_directories().await;
        let valid_path = self.validate_path(file_path, allowed_directories)?;
        validate_file_size(&valid_path, None, max_bytes).await?;
        let kind = mime_from_path(&valid_path)?;
        let content = read_file_as_base64(&valid_path).await?;
        Ok((kind, content))
    }

    // Get file stats
    pub async fn get_file_stats(&self, file_path: &Path) -> ServiceResult<FileInfo> {
        let allowed_directories = self.allowed_directories().await;
        let valid_path = self.validate_path(file_path, allowed_directories)?;

        let metadata = std::fs::metadata(valid_path)?;

        let size = metadata.len();
        let created = metadata.created().ok();
        let modified = metadata.modified().ok();
        let accessed = metadata.accessed().ok();
        let is_directory = metadata.is_dir();
        let is_file = metadata.is_file();

        Ok(FileInfo {
            size,
            created,
            modified,
            accessed,
            is_directory,
            is_file,
            metadata,
        })
    }
}

#[derive(Debug)]
pub struct FileInfo {
    pub size: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub is_directory: bool,
    pub is_file: bool,
    pub metadata: fs::Metadata,
}

impl std::fmt::Display for FileInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r#"size: {}
created: {}
modified: {}
accessed: {}
isDirectory: {}
isFile: {}
permissions: {}
"#,
            self.size,
            self.created.map_or("".to_string(), format_system_time),
            self.modified.map_or("".to_string(), format_system_time),
            self.accessed.map_or("".to_string(), format_system_time),
            self.is_directory,
            self.is_file,
            format_permissions(&self.metadata)
        )
    }
}
