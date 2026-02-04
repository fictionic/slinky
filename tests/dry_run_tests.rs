use std::fs;
mod common;
use common::TestContext;

#[test]
fn test_slinky_dry_run_delete() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("target.txt", "link.txt")?;

    ctx.run_slinky(&["--dry-run", "delete"])
        .success();

    assert!(fs::symlink_metadata(&link).is_ok());
    
    Ok(())
}

#[test]
fn test_slinky_dry_run_tidy() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("foo/../bar", "link.txt")?;

    ctx.run_slinky(&["--dry-run", "tidy"])
        .success();

    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "foo/../bar");
    
    Ok(())
}

#[test]
fn test_slinky_ln_dry_run() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let target = ctx.create_file("target.txt", "content")?;
    let link = ctx.path().join("link.txt");

    ctx.run_slinky_ln(&[target.to_str().unwrap(), link.to_str().unwrap(), "--dry-run"])
        .success();

    assert!(fs::symlink_metadata(&link).is_err());
    
    Ok(())
}

#[test]
fn test_slinky_ln_dry_run_force() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    ctx.create_file("target.txt", "content")?;
    let existing = ctx.create_file("existing.txt", "old content")?;

    ctx.run_slinky_ln(&["target.txt", "existing.txt", "--force", "--dry-run"])
        .success();

    let metadata = fs::symlink_metadata(&existing)?;
    assert!(metadata.is_file());
    assert_eq!(fs::read_to_string(&existing)?, "old content");
    
    Ok(())
}
