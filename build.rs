use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=vendor/b4");

    let source_root = Path::new("vendor/b4/src/b4");
    if !source_root.exists() {
        println!("cargo:warning=vendor/b4 not found, skip b4 compile step");
        return;
    }

    let out_root = match env::var("OUT_DIR") {
        Ok(path) => PathBuf::from(path).join("b4-pyc"),
        Err(err) => {
            println!("cargo:warning=OUT_DIR not available for b4 compile step: {err}");
            return;
        }
    };

    let mut py_files = Vec::new();
    if let Err(err) = collect_py_files(source_root, &mut py_files) {
        println!("cargo:warning=failed to scan b4 source tree: {err}");
        return;
    }

    if py_files.is_empty() {
        println!("cargo:warning=no python files found under vendor/b4/src/b4");
        return;
    }

    py_files.sort();
    let python = env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());

    for file in py_files {
        let relative = match file.strip_prefix(source_root) {
            Ok(path) => path,
            Err(err) => {
                println!(
                    "cargo:warning=failed to relativize {}: {err}",
                    file.display()
                );
                return;
            }
        };

        let output = out_root.join(relative).with_extension("pyc");
        if let Some(parent) = output.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            println!(
                "cargo:warning=failed to create b4 compile output dir {}: {err}",
                parent.display()
            );
            return;
        }

        let status = Command::new(&python)
            .arg("-c")
            .arg("import py_compile,sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], doraise=True)")
            .arg(&file)
            .arg(&output)
            .status();

        match status {
            Ok(code) if code.success() => {}
            Ok(code) => {
                println!(
                    "cargo:warning=b4 compile failed for {} (exit {code})",
                    file.display()
                );
                return;
            }
            Err(err) => {
                println!("cargo:warning=unable to launch {python} for b4 compile step: {err}");
                return;
            }
        }
    }

    println!("cargo:rustc-env=COURIER_B4_COMPILED=1");
}

fn collect_py_files(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_py_files(&path, out)?;
            continue;
        }

        if path.extension().is_some_and(|extension| extension == "py") {
            out.push(path);
        }
    }

    Ok(())
}
