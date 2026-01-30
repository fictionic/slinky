use clap::CommandFactory;
use clap_complete::{generate_to, Shell};
use std::fs;
use std::path::Path;

include!("src/cli.rs");

fn main() -> std::io::Result<()> {
    let out_dir = Path::new("generate");
    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }

    // Generate for slinky
    let mut cmd = SlinkyCli::command();
    let bin_name = "slinky";

    for &shell in &[Shell::Bash, Shell::Fish, Shell::Zsh] {
        generate_to(shell, &mut cmd, bin_name, out_dir)?;
    }

    // Man page for slinky
    let man = clap_mangen::Man::new(cmd);
    let mut buffer = Vec::new();
    man.render(&mut buffer)?;
    fs::write(out_dir.join("slinky.1"), buffer)?;


    // Generate for slinky-ln
    let mut cmd_ln = SlinkyLnCli::command();
    let bin_name_ln = "slinky-ln";

    for &shell in &[Shell::Bash, Shell::Fish, Shell::Zsh] {
        generate_to(shell, &mut cmd_ln, bin_name_ln, out_dir)?;
    }
    
    // Man page for slinky-ln
    let man_ln = clap_mangen::Man::new(cmd_ln);
    let mut buffer_ln = Vec::new();
    man_ln.render(&mut buffer_ln)?;
    fs::write(out_dir.join("slinky-ln.1"), buffer_ln)?;

    println!("cargo:rerun-if-changed=src/cli.rs");
    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}
