use rust_mcp_filesystem::fs_service::FileSystemService;
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn test_from_end_optimization() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.txt");

    // Create a test file with 10 lines
    let content = (1..=10)
        .map(|i| format!("line{}", i))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&file_path, &content).unwrap();

    let allowed_dirs = vec![temp_dir.path().to_string_lossy().to_string()];
    let service = FileSystemService::try_new(&allowed_dirs).unwrap();

    // Test 1: Read last 3 lines
    let result = service.read_file_lines(&file_path, 0, Some(3), true).await.unwrap();
    assert_eq!(result, "line8\nline9\nline10");
    println!("✓ Test 1 passed: Read last 3 lines");

    // Test 2: Read last 5 lines
    let result = service.read_file_lines(&file_path, 0, Some(5), true).await.unwrap();
    assert_eq!(result, "line6\nline7\nline8\nline9\nline10");
    println!("✓ Test 2 passed: Read last 5 lines");

    // Test 3: Skip last 2 lines, read 3 lines (lines 6-8)
    let result = service.read_file_lines(&file_path, 2, Some(3), true).await.unwrap();
    assert_eq!(result, "line6\nline7\nline8");
    println!("✓ Test 3 passed: Skip 2, read 3 from end");

    // Test 4: Read all from end
    let result = service.read_file_lines(&file_path, 0, None, true).await.unwrap();
    assert_eq!(result, content);
    println!("✓ Test 4 passed: Read all from end");

    // Test 5: Offset exceeds total lines
    let result = service.read_file_lines(&file_path, 20, Some(3), true).await.unwrap();
    assert_eq!(result, "");
    println!("✓ Test 5 passed: Offset exceeds total lines");
}

#[tokio::test]
async fn test_from_end_with_trailing_newline() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test2.txt");

    // Create file with trailing newline
    fs::write(&file_path, "line1\nline2\nline3\n").unwrap();

    let allowed_dirs = vec![temp_dir.path().to_string_lossy().to_string()];
    let service = FileSystemService::try_new(&allowed_dirs).unwrap();

    // Read last 2 lines
    let result = service.read_file_lines(&file_path, 0, Some(2), true).await.unwrap();
    assert_eq!(result, "line2\nline3\n");
    println!("✓ Test with trailing newline passed");
}
