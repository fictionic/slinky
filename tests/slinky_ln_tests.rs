use predicates::prelude::*;
use std::fs;
use std::path::Path;

mod common;
use common::TestContext;

#[test]
fn test_create_link_parent_dir_non_existent() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;
    
    let non_existent_parent_dir_link = ctx.path().join("non_existent_dir/link.txt");

    ctx.run_slinky_ln(&[target_file.to_str().unwrap(), non_existent_parent_dir_link.to_str().unwrap()])
        .failure()
        .stderr(predicate::str::contains("No such file or directory"));

    Ok(())
}

#[test]
fn test_create_link_force_overwrite_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target1.txt", "content1")?;
    ctx.create_file("target2.txt", "content2")?;
    
    let existing_symlink = ctx.create_symlink("target1.txt", "existing_link.txt")?; // Link to target1.txt

    ctx.run_slinky_ln(&["target2.txt", "existing_link.txt", "--force"])
        .success();

    // Verify that existing_link.txt is now a symlink pointing to target2.txt
    assert!(existing_symlink.is_symlink());
    assert_eq!(fs::read_link(&existing_symlink)?.to_str().unwrap(), "target2.txt");

    Ok(())
}

#[test]
fn test_create_link_force_overwrite_file() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    ctx.create_file("existing.txt", "old content")?;

    ctx.run_slinky_ln(&["target.txt", "existing.txt", "--force"])
        .success();

    // Verify that existing.txt is now a symlink pointing to target.txt
    let existing_file = ctx.path().join("existing.txt");
    assert!(existing_file.is_symlink());
    assert_eq!(fs::read_link(&existing_file)?.to_str().unwrap(), "target.txt");

    Ok(())
}

#[test]
fn test_create_link_destination_exists_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    
    let existing_symlink = ctx.create_symlink("old_target.txt", "existing_link.txt")?;

    ctx.run_slinky_ln(&["target.txt", "existing_link.txt"])
        .failure()
        .stderr(predicate::str::contains("File exists"));

    // Ensure the existing symlink still points to its old target
    assert_eq!(fs::read_link(&existing_symlink)?.to_str().unwrap(), "old_target.txt");

    Ok(())
}

#[test]
fn test_create_link_destination_exists_file() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    
    let existing_file_path = ctx.create_file("existing.txt", "old content")?;

    ctx.run_slinky_ln(&["target.txt", "existing.txt"])
        .failure()
        .stderr(predicate::str::contains("File exists"));

    // Ensure the existing file content is unchanged
    assert_eq!(fs::read_to_string(&existing_file_path)?, "old content");

    Ok(())
}

#[test]
fn test_create_link() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.path().join("new_link.txt");

    // "some_target" doesn't exist, so we need --allow-dangling
    ctx.run_slinky_ln(&["some_target", link.to_str().unwrap(), "--allow-dangling"])
        .success();

    let target = fs::read_link(link)?;
    assert_eq!(target.to_str().unwrap(), "some_target");

    Ok(())
}

#[test]
fn test_create_link_in_dir() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;

    let dest_dir = ctx.path().join("dest");
    fs::create_dir(&dest_dir)?;

    // Origin is a directory
    ctx.run_slinky_ln(&[target_file.to_str().unwrap(), dest_dir.to_str().unwrap()])
        .success();

    // Should create dest/target.txt
    let expected_link = dest_dir.join("target.txt");
    assert!(expected_link.is_symlink());

    let target = fs::read_link(&expected_link)?;
    assert_eq!(target, target_file);

    Ok(())
}

#[test]
fn test_create_to_file() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;
    let link = ctx.path().join("link.txt");

    ctx.run_slinky_ln(&[target_file.to_str().unwrap(), link.to_str().unwrap()])
        .success();

    let target = fs::read_link(&link)?;
    assert_eq!(target, target_file);

    Ok(())
}

#[test]
fn test_create_missing_target() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let missing_file = ctx.path().join("missing.txt");

    ctx.run_slinky_ln(&[missing_file.to_str().unwrap()])
        .failure()
        .stderr(predicate::str::contains(
            "refusing to create dangling symlink",
        ));

    Ok(())
}

#[test]
fn test_create_implicit_origin_remote() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let subdir = ctx.path().join("subdir");
    fs::create_dir(&subdir)?;
    ctx.create_file("subdir/file.txt", "content")?;

    ctx.run_slinky_ln(&["subdir/file.txt"])
        .success();

    let expected_link = ctx.path().join("file.txt");
    assert!(expected_link.is_symlink());
    let target = fs::read_link(&expected_link)?;
    assert_eq!(target.to_str().unwrap(), "subdir/file.txt");

    Ok(())
}


#[test]
fn test_create_absolute_flag() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;
    let link = ctx.path().join("link.txt");

    ctx.run_slinky_ln(&["target.txt", "link.txt", "--absolute"])
        .success();

    let target = fs::read_link(&link)?;
    assert!(target.is_absolute());
    let canonical = fs::canonicalize(&target_file)?;
    assert_eq!(target, canonical);

    Ok(())
}

#[test]
fn test_create_relative_flag() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let subdir = ctx.path().join("subdir");
    fs::create_dir(&subdir)?;
    ctx.create_file("subdir/target.txt", "content")?;

    let link = ctx.path().join("link.txt");

    ctx.run_slinky_ln(&["subdir/target.txt", "link.txt", "--relative"])
        .success();

    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "subdir/target.txt");

    // Test deeper
    let deep_dir = ctx.path().join("a/b/c");
    fs::create_dir_all(&deep_dir)?;
    let deep_link = deep_dir.join("link.txt");

    ctx.run_slinky_ln(&["subdir/target.txt", "a/b/c/link.txt", "--relative"])
        .success();

    let target3 = fs::read_link(&deep_link)?;
    assert_eq!(target3.to_str().unwrap(), "../../../subdir/target.txt");

    Ok(())
}

#[test]
fn test_create_link_dereference() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;

    ctx.create_symlink("target.txt", "link1.txt")?;

    let link2 = ctx.path().join("link2.txt");

    ctx.run_slinky_ln(&["link1.txt", "link2.txt", "--dereference"])
        .success();

    let target = fs::read_link(&link2)?;
    // Should point to target.txt, not link1.txt
    // Since link1.txt points to "target.txt" (relative), 
    // canonicalize will return the absolute path to target.txt
    let canonical_target = fs::canonicalize(&target_file)?;
    assert_eq!(target, canonical_target);

    Ok(())
}

#[test]
fn test_create_link_dereference_relative() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let subdir = ctx.path().join("subdir");
    fs::create_dir(&subdir)?;
    ctx.create_file("subdir/target.txt", "content")?;

    // link1 -> subdir/target.txt
    ctx.create_symlink("subdir/target.txt", "link1.txt")?;

    // link2 should point to subdir/target.txt (relative) via dereferencing link1
    let link2 = ctx.path().join("link2.txt");

    ctx.run_slinky_ln(&["link1.txt", "link2.txt", "--dereference", "--relative"])
        .success();

    let target = fs::read_link(&link2)?;
    // Should be "subdir/target.txt", not "link1.txt" and not an absolute path
    assert_eq!(target.to_str().unwrap(), "subdir/target.txt");

    Ok(())
}

#[test]
fn test_create_link_dereference_dangling() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    
    // link1 -> missing.txt
    ctx.create_symlink("missing.txt", "link1.txt")?;

    let link2 = ctx.path().join("link2.txt");

    ctx.run_slinky_ln(&["link1.txt", "link2.txt", "--dereference", "--allow-dangling"])
        .success();

    let target = fs::read_link(&link2)?;
    // Should point to "missing.txt" (resolved from link1.txt)
    // Note: our manual resolution joins with parent, so it might be absolute or relative 
    // depending on the original target. In this case, it joined "dir" + "missing.txt".
    assert!(target.to_str().unwrap().ends_with("missing.txt"));

    Ok(())
}


#[test]
fn test_slinky_ln_verbose() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    let link_path = ctx.path().join("link.txt");

    ctx.run_slinky_ln(&["target.txt", link_path.to_str().unwrap(), "--verbose"])
        .success()
        .stdout(predicate::str::contains("create symlink"));

    // Ensure the link was actually created
    assert!(link_path.is_symlink());
    assert_eq!(fs::read_link(&link_path)?, Path::new("target.txt"));

    Ok(())
}

#[test]
fn test_create_link_force_overwrite_directory_fails() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    
    let existing_dir = ctx.path().join("existing_dir");
    fs::create_dir(&existing_dir)?;
    // Create a directory where the link would be created
    fs::create_dir(existing_dir.join("target.txt"))?;

    ctx.run_slinky_ln(&["target.txt", "existing_dir", "--force"])
        .failure()
        .stderr(predicate::str::contains("Is a directory"));

    Ok(())
}

fn check_conflict(
    ctx: &TestContext,
    args: &[&str],
    error_message_substring: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    ctx.run_slinky_ln(&[vec!["target"], args.to_vec()].concat())
        .failure()
        .stderr(predicate::str::contains(error_message_substring));
    Ok(())
}

#[test]
fn test_create_conflict_flags() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    check_conflict(&ctx, &["--absolute", "--allow-dangling"], "cannot be used with")?;
    check_conflict(&ctx, &["--relative", "--allow-dangling"], "cannot be used with")?;
    check_conflict(&ctx, &["--absolute", "--relative"], "cannot be used with")?;
    Ok(())
}

#[test]
fn test_create_symlink_tree() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let source_dir = ctx.path().join("source");
    fs::create_dir(&source_dir)?;
    ctx.create_file("source/file1.txt", "content1")?;
    
    let sub_dir = source_dir.join("subdir");
    fs::create_dir(&sub_dir)?;
    ctx.create_file("source/subdir/file2.txt", "content2")?;

    let dest_dir = ctx.path().join("dest");

    ctx.run_slinky_ln(&["source", "dest", "--tree"])
        .success();

    assert!(dest_dir.is_dir());
    assert!(!fs::symlink_metadata(&dest_dir)?.file_type().is_symlink());

    let link1 = dest_dir.join("file1.txt");
    assert!(fs::symlink_metadata(&link1)?.file_type().is_symlink());
    
    let link2 = dest_dir.join("subdir/file2.txt");
    assert!(fs::symlink_metadata(&link2)?.file_type().is_symlink());

    Ok(())
}

#[test]
fn test_create_hardlink_tree() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let source_dir = ctx.path().join("source");
    fs::create_dir(&source_dir)?;
    let file1 = ctx.create_file("source/file1.txt", "content1")?;
    
    let sub_dir = source_dir.join("subdir");
    fs::create_dir(&sub_dir)?;
    let file2 = ctx.create_file("source/subdir/file2.txt", "content2")?;

    let dest_dir = ctx.path().join("dest");

    ctx.run_slinky_ln(&["source", "dest", "--tree", "--hard"])
        .success();

    assert!(dest_dir.is_dir());
    assert!(!fs::symlink_metadata(&dest_dir)?.file_type().is_symlink());

    let link1 = dest_dir.join("file1.txt");
    assert!(!fs::symlink_metadata(&link1)?.file_type().is_symlink());
    assert_eq!(fs::metadata(&link1)?.len(), fs::metadata(&file1)?.len());
    
    let link2 = dest_dir.join("subdir/file2.txt");
    assert!(!fs::symlink_metadata(&link2)?.file_type().is_symlink());
    assert_eq!(fs::metadata(&link2)?.len(), fs::metadata(&file2)?.len());

    Ok(())
}

#[test]
fn test_create_tree_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    check_conflict(&ctx, &["--tree", "--absolute"], "cannot be used with")?;
    check_conflict(&ctx, &["--tree", "--relative"], "cannot be used with")?;
    check_conflict(&ctx, &["--tree", "--allow-dangling"], "cannot be used with")?;
    Ok(())
}
