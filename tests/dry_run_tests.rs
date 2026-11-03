use std::fs;
mod common;
use common::TestContext;

#[test]
fn test_slinky_dry_run_remove() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("target.txt", "link.txt")?;

    ctx.run_slinky(&["--dry-run", "remove"]).success();

    assert!(fs::symlink_metadata(&link).is_ok());

    Ok(())
}

#[test]
fn test_slinky_dry_run_tidy() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = TestContext::new()?;
    let link = ctx.create_symlink("foo/../bar", "link.txt")?;

    ctx.run_slinky(&["--dry-run", "tidy-target"]).success();

    let target = fs::read_link(&link)?;
    assert_eq!(target.to_str().unwrap(), "foo/../bar");

    Ok(())
}
