use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

mod common;
use common::TestContext;

#[test]
fn test_create_link_parent_dir_non_existent() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;

    let non_existent_parent_dir_link = ctx.path().join("non_existent_dir/link.txt");

    ctx.run_slinky_ln(&[
        target_file.to_str().unwrap(),
        non_existent_parent_dir_link.to_str().unwrap(),
    ])
    .failure()
    .stderr(predicate::str::contains("Origin directory does not exist"));

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
    assert_eq!(
        fs::read_link(&existing_symlink)?.to_str().unwrap(),
        "target2.txt"
    );

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
    assert_eq!(
        fs::read_link(&existing_file)?.to_str().unwrap(),
        "target.txt"
    );

    Ok(())
}

#[test]
fn test_create_link_destination_exists_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;

    let existing_symlink = ctx.create_symlink("old_target.txt", "existing_link.txt")?;

    ctx.run_slinky_ln(&["target.txt", "existing_link.txt"])
        .failure()
        .stderr(predicate::str::contains(
            "Refusing to overwrite existing file at origin",
        ));

    // Ensure the existing symlink still points to its old target
    assert_eq!(
        fs::read_link(&existing_symlink)?.to_str().unwrap(),
        "old_target.txt"
    );

    Ok(())
}

#[test]
fn test_create_link_destination_exists_file() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;

    let existing_file_path = ctx.create_file("existing.txt", "old content")?;

    ctx.run_slinky_ln(&["target.txt", "existing.txt"])
        .failure()
        .stderr(predicate::str::contains(
            "Refusing to overwrite existing file at origin",
        ));

    // Ensure the existing file content is unchanged
    assert_eq!(fs::read_to_string(&existing_file_path)?, "old content");

    Ok(())
}

#[test]
fn test_create_link() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.path().join("new_link.txt");

    // "some_target" doesn't exist, so we need --literal
    ctx.run_slinky_ln(&["some_target", link.to_str().unwrap(), "--literal"])
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

    ctx.run_slinky_ln(&["subdir/file.txt"]).success();

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

    // Should point to target.txt, not link1.txt. The typed path is relative, so the text is
    // relative too
    assert_eq!(fs::read_link(&link2)?, Path::new("target.txt"));
    assert_eq!(
        fs::read_to_string(&link2)?,
        fs::read_to_string(&target_file)?
    );

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

    // the chain ends at a missing file, so there is no file to link to
    ctx.run_slinky_ln(&["link1.txt", "link2.txt", "--dereference"])
        .failure()
        .stderr(predicate::str::contains("Target does not exist"));

    assert!(!link2.is_symlink());

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
        .stderr(predicate::str::contains(
            "Refusing to overwrite existing directory at origin",
        ));

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
    // --literal conflicts with every flag that needs the target to resolve
    for flag in [
        "--absolute",
        "--relative",
        "--dereference",
        "--hard",
        "--tree",
    ] {
        check_conflict(&ctx, &[flag, "--literal"], "cannot be used with")?;
    }
    check_conflict(&ctx, &["--absolute", "--relative"], "cannot be used with")?;
    Ok(())
}

#[test]
fn test_create_link_origin_outside_cwd() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("source/file.txt", "content")?;
    fs::create_dir(ctx.path().join("sub"))?;

    // the target resolves from the current directory, not from sub/
    ctx.run_slinky_ln(&["source/file.txt", "sub/link.txt"])
        .success();

    let link = ctx.path().join("sub/link.txt");
    assert_eq!(fs::read_link(&link)?, Path::new("../source/file.txt"));
    assert_eq!(fs::read_to_string(&link)?, "content");

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

    ctx.run_slinky_ln(&["source", "dest", "--tree"]).success();

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
fn test_create_symlink_tree_origin_outside_cwd() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("source/file.txt", "content")?;
    fs::create_dir(ctx.path().join("sub"))?;

    ctx.run_slinky_ln(&["source", "sub/tree", "--tree"])
        .success();

    let link = ctx.path().join("sub/tree/file.txt");
    assert_eq!(fs::read_link(&link)?, Path::new("../../source/file.txt"));
    assert_eq!(fs::read_to_string(&link)?, "content");

    Ok(())
}

#[test]
fn test_create_link_force_missing_target_preserves_origin() -> Result<(), Box<dyn std::error::Error>>
{
    let ctx = TestContext::new()?;

    // We have an existing file that we are threatening to overwrite
    let existing_file = ctx.create_file("existing.txt", "precious content")?;

    // We try to link to a missing target with --force
    // This should fail because the target is missing (and we didn't say --literal)
    ctx.run_slinky_ln(&["missing_target", "existing.txt", "--force"])
        .failure()
        .stderr(predicate::str::contains("Target does not exist"));

    // Crucially, the existing file should STILL be there because we shouldn't have deleted it yet
    assert!(existing_file.exists());
    assert_eq!(fs::read_to_string(&existing_file)?, "precious content");

    Ok(())
}

#[test]
fn test_create_hardlink_force_directory_target_preserves_origin()
-> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    // Target is a directory that EXISTS -- so the "Target does not exist" guard
    // does not fire; the failure comes from create_hard_link instead.
    let target_dir = ctx.path().join("target_dir");
    fs::create_dir(&target_dir)?;

    let existing_file = ctx.create_file("existing.txt", "precious content")?;

    ctx.run_slinky_ln(&["target_dir", "existing.txt", "--force", "--hard"])
        .failure()
        .stderr(predicate::str::contains("Refusing to hardlink a directory"));

    // The create fails BEFORE the removal, so the origin must survive.
    assert!(existing_file.exists());
    assert_eq!(fs::read_to_string(&existing_file)?, "precious content");

    Ok(())
}

#[test]
fn test_create_hardlink_force_missing_target_preserves_origin()
-> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;

    let existing_file = ctx.create_file("existing.txt", "precious content")?;

    // Hardlinks require the target to exist, even with --force
    ctx.run_slinky_ln(&["missing_target", "existing.txt", "--force", "--hard"])
        .failure()
        .stderr(predicate::str::contains("Target does not exist"));

    assert!(existing_file.exists());
    assert_eq!(fs::read_to_string(&existing_file)?, "precious content");

    Ok(())
}

#[test]
fn test_create_tree_force_missing_target_preserves_origin() -> Result<(), Box<dyn std::error::Error>>
{
    let ctx = TestContext::new()?;

    let existing_dir = ctx.path().join("existing_dir");
    fs::create_dir(&existing_dir)?;
    ctx.create_file("existing_dir/precious.txt", "content")?;

    // Trees require the target to exist
    ctx.run_slinky_ln(&["missing_source", "existing_dir", "--force", "--tree"])
        .failure()
        .stderr(predicate::str::contains("Target does not exist"));

    assert!(existing_dir.exists());
    assert!(existing_dir.join("precious.txt").exists());

    Ok(())
}

#[test]
fn test_create_link_dry_run() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;

    ctx.run_slinky_ln(&["target.txt", "link.txt", "--dry-run"])
        .success();

    assert!(fs::symlink_metadata(ctx.path().join("link.txt")).is_err());

    Ok(())
}

#[test]
fn test_create_link_dry_run_existing_origin_fails() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    let existing_file = ctx.create_file("existing.txt", "precious content")?;

    ctx.run_slinky_ln(&["target.txt", "existing.txt", "--dry-run"])
        .failure()
        .stderr(predicate::str::contains(
            "Refusing to overwrite existing file at origin",
        ));

    assert_eq!(fs::read_to_string(&existing_file)?, "precious content");

    Ok(())
}

#[test]
fn test_create_link_dry_run_force_preserves_origin() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    let existing_file = ctx.create_file("existing.txt", "precious content")?;

    ctx.run_slinky_ln(&[
        "target.txt",
        "existing.txt",
        "--dry-run",
        "--force",
        "--verbose",
    ])
    .success()
    .stdout(predicate::str::contains("Remove existing file at origin"))
    .stdout(predicate::str::contains("create symlink"));

    assert!(!existing_file.is_symlink());
    assert_eq!(fs::read_to_string(&existing_file)?, "precious content");

    Ok(())
}

#[test]
fn test_create_link_literal_keeps_text() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    fs::create_dir(ctx.path().join("sub"))?;

    // the link is not in the current directory, so a rebase would change the text
    ctx.run_slinky_ln(&["some_target", "sub/link.txt", "--literal"])
        .success();

    assert_eq!(
        fs::read_link(ctx.path().join("sub/link.txt"))?,
        Path::new("some_target")
    );

    Ok(())
}

#[test]
fn test_create_link_keeps_symlink_in_target_path() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("data/file.txt", "content")?;
    ctx.create_symlink("data", "slink")?;
    fs::create_dir(ctx.path().join("sub"))?;

    ctx.run_slinky_ln(&["slink/file.txt", "sub/link.txt"])
        .success();
    let link = ctx.path().join("sub/link.txt");
    assert_eq!(fs::read_link(&link)?, Path::new("../slink/file.txt"));
    assert_eq!(fs::read_to_string(&link)?, "content");

    ctx.run_slinky_ln(&["slink/file.txt", "abs_link.txt", "--absolute"])
        .success();
    // the process cwd has no symlinks, so only `slink` can be a symlink in the text
    let cwd = fs::canonicalize(ctx.path())?;
    assert_eq!(
        fs::read_link(ctx.path().join("abs_link.txt"))?,
        cwd.join("slink/file.txt")
    );

    Ok(())
}

#[test]
fn test_create_link_collapses_only_safe_parent() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("source/file.txt", "content")?;
    fs::create_dir(ctx.path().join("realdir"))?;
    ctx.create_symlink("realdir", "slink")?;
    fs::create_dir(ctx.path().join("sub"))?;

    // realdir is a physical directory, so `realdir/..` collapses
    ctx.run_slinky_ln(&["realdir/../source/file.txt", "sub/real.txt"])
        .success();
    let real_link = ctx.path().join("sub/real.txt");
    assert_eq!(fs::read_link(&real_link)?, Path::new("../source/file.txt"));
    assert_eq!(fs::read_to_string(&real_link)?, "content");

    // slink is a symlink, so `slink/..` stays
    ctx.run_slinky_ln(&["slink/../source/file.txt", "sub/sym.txt"])
        .success();
    let sym_link = ctx.path().join("sub/sym.txt");
    assert_eq!(
        fs::read_link(&sym_link)?,
        Path::new("../slink/../source/file.txt")
    );
    assert_eq!(fs::read_to_string(&sym_link)?, "content");

    Ok(())
}

#[test]
fn test_create_hardlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target_file = ctx.create_file("target.txt", "content")?;

    ctx.run_slinky_ln(&["target.txt", "link.txt", "--hard"])
        .success();

    let link = ctx.path().join("link.txt");
    assert!(!link.is_symlink());
    assert_eq!(
        fs::metadata(&link)?.ino(),
        fs::metadata(&target_file)?.ino()
    );

    Ok(())
}

#[test]
fn test_create_link_in_dir_basename_from_canonical_target() -> Result<(), Box<dyn std::error::Error>>
{
    let ctx = TestContext::new()?;
    fs::create_dir(ctx.path().join("dest"))?;

    // `.` has no file name, so the basename comes from the canonical path
    ctx.run_slinky_ln(&[".", "dest"]).success();

    let cwd = fs::canonicalize(ctx.path())?;
    let link = ctx.path().join("dest").join(cwd.file_name().unwrap());
    assert_eq!(fs::read_link(&link)?, Path::new(".."));

    Ok(())
}

#[test]
fn test_create_hardlink_to_symlink_links_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("data/file.txt", "content")?;
    let slink = ctx.create_symlink("data/file.txt", "slink")?;

    ctx.run_slinky_ln(&["slink", "link", "--hard"]).success();

    // without -L, like ln: the new link is another name for the symlink itself
    let link = ctx.path().join("link");
    assert!(link.is_symlink());
    assert_eq!(
        fs::symlink_metadata(&link)?.ino(),
        fs::symlink_metadata(&slink)?.ino()
    );

    Ok(())
}

#[test]
fn test_create_hardlink_tree_from_dot() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let file = ctx.create_file("source/sub/file.txt", "content")?;

    // `.` has no file name
    let mut cmd = ctx.slinky_ln_cmd();
    cmd.current_dir(ctx.path().join("source"));
    cmd.args([".", "../dest", "--hard", "--tree"])
        .assert()
        .success();

    let link = ctx.path().join("dest/sub/file.txt");
    assert!(!link.is_symlink());
    assert_eq!(fs::metadata(&link)?.ino(), fs::metadata(&file)?.ino());

    Ok(())
}

#[test]
fn test_create_hardlink_tree_symlink_trailing_slash() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let file = ctx.create_file("data/file.txt", "content")?;
    ctx.create_symlink("data", "dlink")?;

    // a trailing slash or dot follows dlink, so the log names data, not dlink
    let data_canonical = fs::canonicalize(ctx.path().join("data"))?;
    for (target, dest) in [("dlink/", "dest_slash"), ("dlink/.", "dest_dot")] {
        let mut cmd = ctx.slinky_ln_cmd();
        cmd.env("NO_COLOR", "1");
        cmd.args([target, dest, "--hard", "--tree", "--verbose"])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "{dest} -> {}\n",
                data_canonical.display()
            )));

        let link = ctx.path().join(dest).join("file.txt");
        assert!(!link.is_symlink());
        assert_eq!(fs::metadata(&link)?.ino(), fs::metadata(&file)?.ino());
    }

    Ok(())
}
