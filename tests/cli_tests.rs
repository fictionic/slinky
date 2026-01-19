use assert_cmd::prelude::*; 
use predicates::prelude::*; 
use std::process::Command;
use std::fs;
use std::os::unix::fs::symlink;
use tempfile::tempdir;

#[test]
fn test_list_default() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let file_path = dir.path().join("real.txt");
    fs::write(&file_path, "content")?;
    
    let link_path = dir.path().join("link.txt");
    // Use the filename only to create a relative symlink
    symlink("real.txt", &link_path)?;

    // FIX: Use the assert_cmd::cargo_bin! macro
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("list");
    
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("attached"))
        // This will now match because the link is relative
        .stdout(predicate::str::contains("link.txt -> real.txt"));

    Ok(())
}

#[test]
fn test_filter_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    symlink("non_existent.txt", dir.path().join("broken.txt"))?;
    
    let real = dir.path().join("real.txt");
    fs::write(&real, "")?;
    symlink("real.txt", dir.path().join("valid.txt"))?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("-g").arg("list");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("dangling"))
        .stdout(predicate::str::contains("valid.txt").not());

    Ok(())
}

#[test]
fn test_to_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let real = dir.path().join("real.txt");
    fs::write(&real, "")?;
    
    let link = dir.path().join("link.txt");
    symlink("real.txt", &link)?; 

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("to-absolute");
    
    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert!(target.is_absolute());
    
    Ok(())
}

#[test]
fn test_edit_target_regex() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let link = dir.path().join("link.txt");
    symlink("version-1.0.txt", &link)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path())
       .arg("edit-target")
       .arg(r"1\.0")
       .arg("2.0");

    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "version-2.0.txt");

    Ok(())
}

#[test]
fn test_to_hardlink_tree() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;

    // 1. Create a real directory with a file
    let source_dir = dir.path().join("source");
    fs::create_dir(&source_dir)?;
    let file_path = source_dir.join("data.txt");
    fs::write(&file_path, "heavy data")?;

    // 2. Create a symlink to that directory
    let link_path = dir.path().join("link_to_dir");
    symlink(&source_dir, &link_path)?;

    // 3. Run to-hardlink-tree
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("to-hardlink-tree");
    cmd.assert().success();

    // 4. Verify: link_path should now be a real directory, not a symlink
    let metadata = fs::symlink_metadata(&link_path)?;
    assert!(metadata.is_dir());
    assert!(!metadata.file_type().is_symlink());

    // 5. Verify: The file inside should be a hardlink (same inode)
    let original_inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&file_path)?);
    let new_file_path = link_path.join("data.txt");
    let new_inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&new_file_path)?);

    assert_eq!(original_inode, new_inode, "File was not hardlinked correctly");

    Ok(())
}
