use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_list_default() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let file_path = dir.path().join("real.txt");
    fs::write(&file_path, "content")?;

    let link_path = dir.path().join("link.txt");
    symlink("real.txt", &link_path)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("print")
        .arg("--status");

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
    cmd.arg("for-each").arg(dir.path()).arg("-x").arg("print").arg("--status");

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
    cmd.arg("for-each").arg(dir.path()).arg("to-absolute");

    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert!(target.is_absolute());

    Ok(())
}

#[test]
fn test_edit_target_regex() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    // Create the target file so it's not dangling
    fs::write(dir.path().join("version-1.0.txt"), "")?;
    let link = dir.path().join("link.txt");
    symlink("version-1.0.txt", &link)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("edit-target")
        .arg(r"1\.0")
        .arg("2.0");

    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "version-2.0.txt");

    Ok(())
}

#[test]
fn test_edit_target_replace_all() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    // Create the target file so it's not dangling
    fs::write(dir.path().join("a-a.txt"), "")?;
    let link = dir.path().join("link.txt");
    symlink("a-a.txt", &link)?;

    // Without --replace-all (default: replace first)
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("edit-target")
        .arg("a")
        .arg("b");
    cmd.assert().success();
    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "b-a.txt");

    // Reset the link for the next test
    fs::remove_file(&link)?;
    symlink("a-a.txt", &link)?;

    // With -g (short for --replace-all)
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("edit-target")
        .arg("a")
        .arg("b")
        .arg("-g");
    cmd.assert().success();
    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "b-b.txt");

    Ok(())
}

#[test]
fn test_edit_target_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let link = dir.path().join("link.txt");
    // Create a dangling symlink
    symlink("broken-1.0.txt", &link)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("edit-target")
        .arg("1.0")
        .arg("2.0");

    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "broken-2.0.txt");

    Ok(())
}

#[test]
fn test_to_hardlink_tree() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;

    let source_dir = dir.path().join("source");
    fs::create_dir(&source_dir)?;
    let file_path = source_dir.join("data.txt");
    fs::write(&file_path, "heavy data")?;

    let link_path = dir.path().join("link_to_dir");
    symlink(&source_dir, &link_path)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each").arg(dir.path()).arg("to-hardlink-tree");
    cmd.assert().success();

    let metadata = fs::symlink_metadata(&link_path)?;
    assert!(metadata.is_dir());
    assert!(!metadata.file_type().is_symlink());

    let original_inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&file_path)?);
    let new_file_path = link_path.join("data.txt");
    let new_inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&new_file_path)?);

    assert_eq!(
        original_inode, new_inode,
        "File was not hardlinked correctly"
    );

    Ok(())
}

#[test]
fn test_filter_origin() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    
    let link1 = dir.path().join("match_this.txt");
    symlink("target.txt", &link1)?;
    
    let link2 = dir.path().join("ignore_this.txt");
    symlink("target.txt", &link2)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each").arg(dir.path()).arg("-o").arg("match").arg("print");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("match_this.txt"))
        .stdout(predicate::str::contains("ignore_this.txt").not());

    Ok(())
}

#[test]
fn test_tidy() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    
    // Test relative link with redundancy
    let rel_link = dir.path().join("rel_link.txt");
    symlink("foo/bar/../baz/./qux", &rel_link)?;
    
    // Test absolute link with redundancy
    let abs_link = dir.path().join("abs_link.txt");
    symlink("/usr/local/../bin/./slinky", &abs_link)?;

    // Test leading .. in relative link (should be preserved)
    let leading_link = dir.path().join("leading_link.txt");
    symlink("../../foo/bar", &leading_link)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each").arg(dir.path()).arg("tidy");
    cmd.assert().success();

    assert_eq!(fs::read_link(rel_link)?.to_str().unwrap(), "foo/baz/qux");
    assert_eq!(fs::read_link(abs_link)?.to_str().unwrap(), "/usr/bin/slinky");
    assert_eq!(fs::read_link(leading_link)?.to_str().unwrap(), "../../foo/bar");

    Ok(())
}

#[test]
fn test_filter_relative_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;

    let rel_link = dir.path().join("rel.txt");
    symlink("target.txt", &rel_link)?;

    let abs_link = dir.path().join("abs.txt");
    symlink("/tmp/target.txt", &abs_link)?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("--only-relative")
        .arg("print");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("rel.txt"))
        .stdout(predicate::str::contains("abs.txt").not());

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("for-each")
        .arg(dir.path())
        .arg("--only-absolute")
        .arg("print");
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
    cmd.arg("for-each").arg(dir.path()).arg("delete");
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
    // "some_target" doesn't exist, so we need --allow-dangling
    cmd.arg("create")
        .arg("some_target")
        .arg(link.to_str().unwrap())
        .arg("--allow-dangling");
    cmd.assert().success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "some_target");

    Ok(())
}

#[test]
fn test_create_link_in_dir() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let target_file = dir.path().join("target.txt");
    fs::write(&target_file, "content")?;

    let dest_dir = dir.path().join("dest");
    fs::create_dir(&dest_dir)?;

    // Origin is a directory
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("create")
        .arg(target_file.to_str().unwrap())
        .arg(dest_dir.to_str().unwrap());
    cmd.assert().success();

    // Should create dest/target.txt
    let expected_link = dest_dir.join("target.txt");
    assert!(expected_link.is_symlink());

    let target = fs::read_link(&expected_link)?;
    assert_eq!(target, target_file);

    Ok(())
}

#[test]
fn test_create_to_file() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let target_file = dir.path().join("target.txt");
    fs::write(&target_file, "content")?;
    let link = dir.path().join("link.txt");

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("create")
        .arg(target_file.to_str().unwrap())
        .arg(link.to_str().unwrap());
    cmd.assert().success();

    let target = fs::read_link(&link)?;
    assert_eq!(target, target_file);

    Ok(())
}

#[test]
fn test_create_missing_target() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let missing_file = dir.path().join("missing.txt");

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("create").arg(missing_file.to_str().unwrap());

    cmd.assert().failure().stderr(predicate::str::contains(
        "refusing to create dangling symlink",
    ));

    Ok(())
}

#[test]
fn test_create_implicit_origin_remote() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir)?;
    let target_file = subdir.join("file.txt");
    fs::write(&target_file, "content")?;

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.current_dir(dir.path());
    cmd.arg("create").arg("subdir/file.txt");

    cmd.assert().success();

    let expected_link = dir.path().join("file.txt");
    assert!(expected_link.is_symlink());
    let target = fs::read_link(&expected_link)?;
    assert_eq!(target.to_str().unwrap(), "subdir/file.txt");

    Ok(())
}

#[test]
fn test_create_absolute_flag() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let target_file = dir.path().join("target.txt");
    fs::write(&target_file, "content")?;
    let link = dir.path().join("link.txt");

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.current_dir(dir.path());
    cmd.arg("create")
        .arg("target.txt")
        .arg("link.txt")
        .arg("--absolute");
    cmd.assert().success();

    let target = fs::read_link(&link)?;
    assert!(target.is_absolute());
    let canonical = fs::canonicalize(&target_file)?;
    assert_eq!(target, canonical);

    Ok(())
}

#[test]
fn test_create_relative_flag() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let subdir = dir.path().join("subdir");
    fs::create_dir(&subdir)?;
    let target_file = subdir.join("target.txt");
    fs::write(&target_file, "content")?;

    let link = dir.path().join("link.txt");

    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.current_dir(dir.path());
    cmd.arg("create")
        .arg("subdir/target.txt")
        .arg("link.txt")
        .arg("--relative");
    cmd.assert().success();

    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "subdir/target.txt");

    // Test deeper
    let deep_dir = dir.path().join("a/b/c");
    fs::create_dir_all(&deep_dir)?;
    let deep_link = deep_dir.join("link.txt");

    let mut cmd2 = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd2.current_dir(dir.path());
    cmd2.arg("create")
        .arg("subdir/target.txt")
        .arg("a/b/c/link.txt")
        .arg("--relative");
    cmd2.assert().success();

    let target2 = fs::read_link(&deep_link)?;
    assert_eq!(target2.to_str().unwrap(), "../../../subdir/target.txt");

    Ok(())
}

#[test]
fn test_create_conflict_flags() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd.arg("create")
        .arg("target")
        .arg("--absolute")
        .arg("--allow-dangling");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));

    let mut cmd2 = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd2.arg("create")
        .arg("target")
        .arg("--relative")
        .arg("--allow-dangling");

    cmd2.assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));

    let mut cmd3 = Command::new(assert_cmd::cargo_bin!("slinky"));
    cmd3.arg("create")
        .arg("target")
        .arg("--absolute")
        .arg("--relative");

    cmd3.assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));

    Ok(())
}
