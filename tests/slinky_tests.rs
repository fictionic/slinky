use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::symlink;

mod common;
use common::TestContext;

#[test]
fn test_filter_target() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    
    ctx.create_symlink("target_foo.txt", "link1.txt")?;
    ctx.create_symlink("target_bar.txt", "link2.txt")?;
    ctx.create_symlink("another_target_foo.txt", "link3.txt")?;

    ctx.run_slinky(&["-t", "foo", "list"])
        .success()
        .stdout(predicate::str::contains("link1.txt"))
        .stdout(predicate::str::contains("link3.txt"))
        .stdout(predicate::str::contains("link2.txt").not());

    Ok(())
}

#[test]
fn test_list_no_dangling_flag() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_symlink("non_existent.txt", "broken.txt")?;

    ctx.create_file("real.txt", "")?;
    ctx.create_symlink("real.txt", "valid.txt")?;

    ctx.run_slinky(&["--only-attached", "list", "--status"])
        .success()
        .stdout(predicate::str::contains("valid.txt"))
        .stdout(predicate::str::contains("broken.txt").not());

    Ok(())
}

#[test]
fn test_list_non_existent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let non_existent_dir = ctx.path().join("non_existent");

    ctx.run_slinky(&[non_existent_dir.to_str().unwrap(), "list"])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_list_only_regular_files() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("file1.txt", "content1")?;
    ctx.create_file("file2.txt", "content2")?;

    ctx.run_slinky(&["list"])
        .success()
        .stdout(predicate::str::is_empty());

    Ok(())
}

#[test]
fn test_list_empty_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    ctx.run_slinky(&["list"])
        .success()
        .stdout(predicate::str::is_empty());

    Ok(())
}

#[test]
fn test_list_default() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "content")?;
    ctx.create_symlink("real.txt", "link.txt")?;

    ctx.run_slinky(&["list", "--status"])
        .success()
        .stdout(predicate::str::contains("attached"))
        .stdout(predicate::str::contains("link.txt -> real.txt"));

    Ok(())
}

#[test]
fn test_list_origin_only() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "content")?;
    ctx.create_symlink("real.txt", "link.txt")?;

    ctx.run_slinky(&["list", "--origin-only"])
        .success()
        .stdout(predicate::str::contains("link.txt"))
        .stdout(predicate::str::contains("->").not())
        .stdout(predicate::str::contains("real.txt").not());

    Ok(())
}

#[test]
fn test_list_alias() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_symlink("real.txt", "link.txt")?;

    ctx.run_slinky(&["ls"])
        .success()
        .stdout(predicate::str::contains("link.txt -> real.txt"));

    Ok(())
}

#[test]
fn test_filter_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_symlink("non_existent.txt", "broken.txt")?;

    ctx.create_file("real.txt", "")?;
    ctx.create_symlink("real.txt", "valid.txt")?;

    ctx.run_slinky(&["-x", "list", "--status"])
        .success()
        .stdout(predicate::str::contains("dangling"))
        .stdout(predicate::str::contains("valid.txt").not());

    Ok(())
}

#[test]
fn test_to_absolute_dangling_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let dangling_link = ctx.create_symlink("non_existent.txt", "dangling.txt")?;

    ctx.run_slinky(&["to-absolute"])
        .success();

    // The dangling symlink should still be dangling and point to the same target
    let target = fs::read_link(&dangling_link)?;
    assert_eq!(target.to_str().unwrap(), "non_existent.txt");

    Ok(())
}

#[test]
fn test_to_absolute_non_existent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let non_existent_dir = ctx.path().join("non_existent");

    ctx.run_slinky(&[non_existent_dir.to_str().unwrap(), "to-absolute"])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_to_absolute_already_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let real_file = ctx.create_file("real.txt", "content")?;
    
    let abs_link = ctx.path().join("absolute_link.txt");
    symlink(fs::canonicalize(&real_file)?, &abs_link)?;

    ctx.run_slinky(&["to-absolute"])
        .success();

    let target = fs::read_link(&abs_link)?;
    assert!(target.is_absolute());
    assert_eq!(target, fs::canonicalize(&real_file)?);

    Ok(())
}

#[test]
fn test_to_absolute_no_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("file.txt", "content")?;

    ctx.run_slinky(&["to-absolute"])
        .success();

    Ok(())
}

#[test]
fn test_to_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "")?;
    let link = ctx.create_symlink("real.txt", "link.txt")?;

    ctx.run_slinky(&["to-absolute"])
        .success();

    let target = fs::read_link(link)?;
    assert!(target.is_absolute());

    Ok(())
}

#[test]
fn test_edit_target_make_valid() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "content")?;
    let link = ctx.create_symlink("broken.txt", "link.txt")?; // Start as dangling

    ctx.run_slinky(&["edit-target", "broken", "real"])
        .success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "real.txt");
    assert!(ctx.path().join(target).exists());

    Ok(())
}

#[test]
fn test_edit_target_make_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "content")?;
    let link = ctx.create_symlink("real.txt", "link.txt")?;

    ctx.run_slinky(&["edit-target", "real", "non_existent"])
        .success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "non_existent.txt");
    assert!(!ctx.path().join(target).exists());

    Ok(())
}

#[test]
fn test_edit_target_non_existent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let non_existent_dir = ctx.path().join("non_existent");

    ctx.run_slinky(&[non_existent_dir.to_str().unwrap(), "edit-target", "pattern", "replacement"])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_edit_target_no_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("file.txt", "content")?;

    ctx.run_slinky(&["edit-target", "pattern", "replacement"])
        .success();

    Ok(())
}

#[test]
fn test_edit_target_no_match() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("original-target.txt", "link.txt")?;

    ctx.run_slinky(&["edit-target", "non_matching_pattern", "new_value"])
        .success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "original-target.txt");

    Ok(())
}

#[test]
fn test_edit_target_regex() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    // Create the target file so it's not dangling
    ctx.create_file("version-1.0.txt", "")?;
    let link = ctx.create_symlink("version-1.0.txt", "link.txt")?;

    ctx.run_slinky(&["edit-target", r"1\.0", "2.0"])
        .success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "version-2.0.txt");

    Ok(())
}

#[test]
fn test_edit_target_replace_all() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    // Create the target file so it's not dangling
    ctx.create_file("a-a.txt", "")?;
    let link_path = ctx.create_symlink("a-a.txt", "link.txt")?;

    // Without --replace-all (default: replace first)
    ctx.run_slinky(&["edit-target", "a", "b"])
        .success();
    let target = fs::read_link(&link_path)?;
    assert_eq!(target.to_str().unwrap(), "b-a.txt");

    // Reset the link for the next test
    fs::remove_file(&link_path)?;
    symlink("a-a.txt", &link_path)?;

    // With -g (short for --replace-all)
    ctx.run_slinky(&["edit-target", "a", "b", "-g"])
        .success();
    let target = fs::read_link(&link_path)?;
    assert_eq!(target.to_str().unwrap(), "b-b.txt");

    Ok(())
}

#[test]
fn test_edit_target_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("broken-1.0.txt", "link.txt")?; // Create a dangling symlink

    ctx.run_slinky(&["edit-target", "1.0", "2.0"])
        .success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "broken-2.0.txt");

    Ok(())
}

#[test]
fn test_to_tree_hard_dangling_dir_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let dangling_link = ctx.create_symlink("non_existent_dir", "dangling_dir")?;

    ctx.run_slinky(&["to-tree", "--hard"])
        .success();

    // The dangling symlink should still exist and be dangling
    assert!(dangling_link.is_symlink());
    assert_eq!(fs::read_link(&dangling_link)?.to_str().unwrap(), "non_existent_dir");

    Ok(())
}

#[test]
fn test_to_tree_hard_non_existent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let non_existent_dir = ctx.path().join("non_existent");

    ctx.run_slinky(&[non_existent_dir.to_str().unwrap(), "to-tree", "--hard"])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_to_tree_hard_symlinks_to_files() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "content")?;
    
    let link_path = ctx.create_symlink("real.txt", "link_to_file.txt")?;

    ctx.run_slinky(&["to-tree", "--hard"])
        .success();

    // The symlink to file should still exist and be a symlink
    assert!(link_path.is_symlink());
    assert_eq!(fs::read_link(&link_path)?.to_str().unwrap(), "real.txt");

    Ok(())
}

#[test]
fn test_to_tree_hard_no_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("file.txt", "content")?;

    ctx.run_slinky(&["to-tree", "--hard"])
        .success();

    Ok(())
}

#[test]
fn test_to_tree_hard() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    let source_dir = ctx.path().join("source");
    fs::create_dir(&source_dir)?;
    let file_path = ctx.create_file("source/data.txt", "heavy data")?;

    let link_path = ctx.create_symlink("source", "link_to_dir")?;

    ctx.run_slinky(&["to-tree", "--hard"])
        .success();

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
    let ctx = TestContext::new()?;
    
    ctx.create_symlink("target.txt", "match_this.txt")?;
    ctx.create_symlink("target.txt", "ignore_this.txt")?;

    ctx.run_slinky(&["-o", "match", "list"])
        .success()
        .stdout(predicate::str::contains("match_this.txt"))
        .stdout(predicate::str::contains("ignore_this.txt").not());

    Ok(())
}

#[test]
fn test_tidy_dangling_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let dangling_link = ctx.create_symlink("non_existent/foo/../bar", "dangling.txt")?; // A "dangling" link with redundancy

    ctx.run_slinky(&["tidy"])
        .success();

    // The dangling symlink should be tidied
    let target = fs::read_link(&dangling_link)?;
    assert_eq!(target.to_str().unwrap(), "non_existent/bar");

    Ok(())
}

#[test]
fn test_tidy_non_existent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let non_existent_dir = ctx.path().join("non_existent");

    ctx.run_slinky(&[non_existent_dir.to_str().unwrap(), "tidy"])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_tidy_already_tidy() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("target.txt", "link.txt")?;

    ctx.run_slinky(&["tidy"])
        .success();

    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "target.txt");

    Ok(())
}

#[test]
fn test_tidy_no_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("file.txt", "content")?;

    ctx.run_slinky(&["tidy"])
        .success();

    Ok(())
}

#[test]
fn test_tidy() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    
    // Test relative link with redundancy
    let rel_link = ctx.create_symlink("foo/bar/../baz/./qux", "rel_link.txt")?;
    
    // Test absolute link with redundancy
    let abs_link = ctx.create_symlink("/usr/local/../bin/./slinky", "abs_link.txt")?;

    // Test leading .. in relative link (should be preserved)
    let leading_link = ctx.create_symlink("../../foo/bar", "leading_link.txt")?;

    ctx.run_slinky(&["tidy"])
        .success();

    assert_eq!(fs::read_link(rel_link)?.to_str().unwrap(), "foo/baz/qux");
    assert_eq!(fs::read_link(abs_link)?.to_str().unwrap(), "/usr/bin/slinky");
    assert_eq!(fs::read_link(leading_link)?.to_str().unwrap(), "../../foo/bar");

    Ok(())
}

#[test]
fn test_filter_relative_absolute() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    ctx.create_symlink("target.txt", "rel.txt")?;
    ctx.create_symlink("/tmp/target.txt", "abs.txt")?;

    ctx.run_slinky(&["--only-relative", "list"])
        .success()
        .stdout(predicate::str::contains("rel.txt"))
        .stdout(predicate::str::contains("abs.txt").not());

    ctx.run_slinky(&["--only-absolute", "list"])
        .success()
        .stdout(predicate::str::contains("abs.txt"))
        .stdout(predicate::str::contains("rel.txt").not());

    Ok(())
}

#[test]
fn test_remove_non_existent_directory() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let non_existent_dir = ctx.path().join("non_existent");

    ctx.run_slinky(&[non_existent_dir.to_str().unwrap(), "remove"])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_remove_regular_file() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("regular_file.txt", "content")?;

    ctx.run_slinky(&["remove"])
        .success();

    Ok(())
}

#[test]
fn test_remove_non_existent_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    ctx.run_slinky(&["remove"])
        .success();

    Ok(())
}

#[test]
fn test_remove_no_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("file.txt", "content")?;

    ctx.run_slinky(&["remove"])
        .success();

    Ok(())
}

#[test]
fn test_remove() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("target.txt", "link.txt")?;

    ctx.run_slinky(&["remove"])
        .success();

    assert!(!link.exists());
    assert!(!fs::symlink_metadata(link).is_ok());

    Ok(())
}

#[test]
fn test_to_tree() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    // Create a source directory structure
    let source_dir = ctx.path().join("source");
    fs::create_dir(&source_dir)?;
    let file1 = ctx.create_file("source/file1.txt", "content1")?;
    
    let sub_dir = source_dir.join("subdir");
    fs::create_dir(&sub_dir)?;
    let file2 = ctx.create_file("source/subdir/file2.txt", "content2")?;

    // Create a symlink to the source directory
    let link_path = ctx.create_symlink("source", "link_to_dir")?;

    // Run slinky to-tree
    ctx.run_slinky(&["to-tree"])
        .success();

    // Verify link_path is now a directory
    let metadata = fs::symlink_metadata(&link_path)?;
    assert!(metadata.is_dir());
    assert!(!metadata.file_type().is_symlink());

    // Verify file1 is a symlink pointing to the original file
    let link_file1 = link_path.join("file1.txt");
    let metadata_file1 = fs::symlink_metadata(&link_file1)?;
    assert!(metadata_file1.file_type().is_symlink());
    
    let target_file1 = fs::read_link(&link_file1)?;
    assert_eq!(target_file1, fs::canonicalize(&file1)?);

    // Verify subdir exists and is a directory
    let link_subdir = link_path.join("subdir");
    let metadata_subdir = fs::symlink_metadata(&link_subdir)?;
    assert!(metadata_subdir.is_dir());
    assert!(!metadata_subdir.file_type().is_symlink());

    // Verify file2 inside subdir is a symlink
    let link_file2 = link_subdir.join("file2.txt");
    let metadata_file2 = fs::symlink_metadata(&link_file2)?;
    assert!(metadata_file2.file_type().is_symlink());

    let target_file2 = fs::read_link(&link_file2)?;
    assert_eq!(target_file2, fs::canonicalize(&file2)?);

    Ok(())
}

#[test]
fn test_to_tree_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let dangling_link = ctx.create_symlink("non_existent_dir", "dangling_dir")?;

    ctx.run_slinky(&["to-tree"])
        .success();

    // The dangling symlink should still exist and be dangling
    assert!(dangling_link.is_symlink());
    assert_eq!(fs::read_link(&dangling_link)?.to_str().unwrap(), "non_existent_dir");

    Ok(())
}

#[test]
fn test_to_tree_file_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("real.txt", "content")?;
    
    let link_path = ctx.create_symlink("real.txt", "link_to_file.txt")?;

    ctx.run_slinky(&["to-tree"])
        .success();

    // The symlink to file should still exist and be a symlink
    assert!(link_path.is_symlink());
    assert_eq!(fs::read_link(&link_path)?.to_str().unwrap(), "real.txt");

    Ok(())
}
