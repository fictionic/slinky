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

#[test]
fn test_filter_relative_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    
    let rel_link = dir.path().join("rel.txt");
    symlink("target.txt", &rel_link)?;
    
    let abs_link = dir.path().join("abs.txt");
    symlink("/tmp/target.txt", &abs_link)?;

    // Test only-relative
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("--only-relative").arg("list");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("rel.txt"))
        .stdout(predicate::str::contains("abs.txt").not());

    // Test only-absolute
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("--only-absolute").arg("list");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("abs.txt"))
        .stdout(predicate::str::contains("rel.txt").not());

    Ok(())
}

#[test]
fn test_delete() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let link = dir.path().join("link.txt");
    symlink("target.txt", &link)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg(dir.path()).arg("delete");
    cmd.assert().success();

    assert!(!link.exists());
    assert!(!fs::symlink_metadata(link).is_ok());

    Ok(())
}

#[test]
fn test_create_link() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let link = dir.path().join("new_link.txt");

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("create-link").arg("some_target").arg(link.to_str().unwrap());
    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "some_target");

    Ok(())
}

#[test]
fn test_link_to_file() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let target_file = dir.path().join("target.txt");
    fs::write(&target_file, "content")?;
    let link = dir.path().join("link.txt");

    // Explicit origin
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("link-to-file").arg(target_file.to_str().unwrap()).arg(link.to_str().unwrap());
    cmd.assert().success();

    let target = fs::read_link(&link)?;
    // On many systems, create_link with absolute path stores absolute path, or relative if provided relative.
    // Here we provided absolute string, so it should match.
    // Wait, target_file.to_str() is absolute path usually from tempdir.
    assert_eq!(target, target_file);

    Ok(())
}

#[test]
fn test_link_to_file_missing_target() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let missing_file = dir.path().join("missing.txt");
    
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("link-to-file").arg(missing_file.to_str().unwrap());
    
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Target file does not exist"));

    Ok(())
}

#[test]
fn test_link_to_file_implicit_origin_remote() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir)?;
    let target_file = subdir.join("file.txt");
    fs::write(&target_file, "content")?;
    
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.current_dir(dir.path());
    // target provided relative
    cmd.arg("link-to-file").arg("subdir/file.txt");
    
    cmd.assert().success();

    let expected_link = dir.path().join("file.txt");
    assert!(expected_link.is_symlink());
    let target = fs::read_link(&expected_link)?;
    assert_eq!(target.to_str().unwrap(), "subdir/file.txt");
    
    Ok(())
}

#[test]
fn test_link_to_file_absolute_flag() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let target_file = dir.path().join("target.txt");
    fs::write(&target_file, "content")?;
    let link = dir.path().join("link.txt");

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.current_dir(dir.path());
    // target relative
    cmd.arg("link-to-file").arg("target.txt").arg("link.txt").arg("--absolute");
    cmd.assert().success();

    let target = fs::read_link(&link)?;
    assert!(target.is_absolute());
    // Should be canonical path
    let canonical = fs::canonicalize(&target_file)?;
    assert_eq!(target, canonical);

    Ok(())
}
