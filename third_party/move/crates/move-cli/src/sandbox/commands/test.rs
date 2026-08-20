// Copyright (c) The Diem Core Contributors
// Copyright (c) The Move Contributors
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::print_stdout)]
use crate::{DEFAULT_BUILD_DIR, DEFAULT_STORAGE_DIR};

use move_command_line_common::{
    env::read_bool_env_var,
    files::{find_filenames, path_to_string},
    testing::{EXP_EXT, add_update_baseline_fix, format_diff, read_env_update_baseline},
};
use move_compiler::command_line::COLOR_MODE_ENV_VAR;
use move_coverage::coverage_map::{CoverageMap, ExecCoverageMapWithModules};
use move_package::{
    BuildConfig,
    compilation::{compiled_package::OnDiskCompiledPackage, package_layout::CompiledPackageLayout},
    resolution::resolution_graph::ResolvedGraph,
    source_package::{layout::SourcePackageLayout, manifest_parser::parse_move_manifest_from_file},
};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    fmt::Write as FmtWrite,
    fs::{self, File},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
};
use tempfile::tempdir;

/// Basic datatest testing framework for the CLI. The `run_one` entrypoint expects
/// an `args.txt` file with arguments that the `move` binary understands (one set
/// of arguments per line). The testing framework runs the commands, compares the
/// result to the expected output, and runs `move clean` to discard resources,
/// modules, and event data created by running the test.///
/// If this env var is set, `move clean` will not be run after each test.
/// this is useful if you want to look at the `storage` or `move_events`
/// produced by a test. However, you'll have to manually run `move clean`
/// before re-running the test.
const NO_MOVE_CLEAN: &str = "NO_MOVE_CLEAN";

/// The filename that contains the arguments to the Move binary.
pub const TEST_ARGS_FILENAME: &str = "args.txt";

/// Name of the environment variable we need to set in order to get tracing
/// enabled in the move VM.
const MOVE_VM_TRACING_ENV_VAR_NAME: &str = "MOVE_VM_TRACE";

/// The default file name (inside the build output dir) for the runtime to
/// dump the execution trace to. The trace will be used by the coverage tool
/// if --track-cov is set. If --track-cov is not set, then no trace file will
/// be produced.
const DEFAULT_TRACE_FILE: &str = "trace";

fn collect_coverage(
    trace_file: &Path,
    build_dir: &Path,
) -> anyhow::Result<ExecCoverageMapWithModules> {
    let canonical_build = build_dir.canonicalize().unwrap();
    let package_name = parse_move_manifest_from_file(
        &SourcePackageLayout::try_find_root(&canonical_build).unwrap(),
    )?
    .package
    .name
    .to_string();
    let pkg = OnDiskCompiledPackage::from_path(
        &build_dir
            .join(package_name)
            .join(CompiledPackageLayout::BuildInfo.path()),
    )?
    .into_compiled_package()?;
    let src_modules = pkg
        .all_modules()
        .map(|unit| {
            let absolute_path = path_to_string(&unit.source_path.canonicalize()?)?;
            Ok((absolute_path, unit.unit.module.clone()))
        })
        .collect::<anyhow::Result<HashMap<_, _>>>()?;

    // build the filter
    let mut filter = BTreeMap::new();
    for (entry, module) in src_modules.into_iter() {
        let module_id = module.self_id();
        filter
            .entry(*module_id.address())
            .or_insert_with(BTreeMap::new)
            .insert(module_id.name().to_owned(), (entry, module));
    }

    // collect filtered trace
    let coverage_map = CoverageMap::from_trace_file(trace_file)
        .to_unified_exec_map()
        .into_coverage_map_with_modules(filter);

    Ok(coverage_map)
}

fn determine_package_nest_depth(
    resolution_graph: &ResolvedGraph,
    pkg_dir: &Path,
) -> anyhow::Result<usize> {
    let mut depth = 0;
    for dep in resolution_graph.package_table.values() {
        depth = std::cmp::max(
            depth,
            dep.package_path.strip_prefix(pkg_dir)?.components().count() + 1,
        );
    }
    Ok(depth)
}

fn pad_tmp_path(tmp_dir: &Path, pad_amount: usize) -> anyhow::Result<PathBuf> {
    let mut tmp_dir = tmp_dir.to_path_buf();
    for i in 0..pad_amount {
        tmp_dir.push(format!("{}", i));
    }
    std::fs::create_dir_all(&tmp_dir)?;
    Ok(tmp_dir)
}

// We need to copy dependencies over (transitively) and at the same time keep the paths valid in
// the package. To do this we compute the resolution graph for all possible dependencies (so in dev
// mode) and then calculate the nesting under `tmp_dir` the we need to copy the root package so
// that it, and all its dependencies reside under `tmp_dir` with the same paths as in the original
// package manifest.
fn copy_deps(tmp_dir: &Path, pkg_dir: &Path) -> anyhow::Result<PathBuf> {
    // Sometimes we run a test that isn't a package for metatests so if there isn't a package we
    // don't need to nest at all. Resolution graph diagnostics are only needed for CLI commands so
    // ignore them by passing a vector as the writer.
    let package_resolution = match (BuildConfig {
        dev_mode: true,
        ..Default::default()
    })
    .resolution_graph_for_package(pkg_dir, &mut Vec::new())
    {
        Ok(pkg) => pkg,
        Err(_) => return Ok(tmp_dir.to_path_buf()),
    };
    let package_nest_depth = determine_package_nest_depth(&package_resolution, pkg_dir)?;
    let tmp_dir = pad_tmp_path(tmp_dir, package_nest_depth)?;
    for dep in package_resolution.package_table.values() {
        let source_dep_path = &dep.package_path;
        let dest_dep_path = tmp_dir.join(dep.package_path.strip_prefix(pkg_dir).unwrap());
        if !dest_dep_path.exists() {
            fs::create_dir_all(&dest_dep_path)?;
        }
        simple_copy_dir(&dest_dep_path, source_dep_path)?;
    }
    Ok(tmp_dir)
}

fn simple_copy_dir(dst: &Path, src: &Path) -> io::Result<()> {
    for entry in fs::read_dir(src)? {
        let src_entry = entry?;
        let src_entry_path = src_entry.path();
        let dst_entry_path = dst.join(src_entry.file_name());
        if src_entry_path.is_dir() {
            fs::create_dir_all(&dst_entry_path)?;
            simple_copy_dir(&dst_entry_path, &src_entry_path)?;
        } else if matches!(
            src_entry_path.extension().and_then(|ext| ext.to_str()),
            Some("md" | "move" | "toml" | "txt")
        ) {
            let contents = fs::read_to_string(&src_entry_path)?;
            fs::write(&dst_entry_path, contents.replace("\r\n", "\n"))?;
        } else {
            fs::copy(&src_entry_path, &dst_entry_path)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn success_exit_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

#[cfg(windows)]
fn success_exit_status() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

fn successful_output(stdout: Vec<u8>) -> Output {
    Output {
        status: success_exit_status(),
        stdout,
        stderr: Vec::new(),
    }
}

fn relative_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    fn visit(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
        for entry in fs::read_dir(current)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(root, &path, files)?;
            } else {
                files.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn files_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let left = fs::read(left)?;
    let right = fs::read(right)?;
    Ok(match (String::from_utf8(left), String::from_utf8(right)) {
        (Ok(left), Ok(right)) => left.replace("\r\n", "\n") == right.replace("\r\n", "\n"),
        (Err(left), Err(right)) => left.as_bytes() == right.as_bytes(),
        _ => false,
    })
}

fn external_command_output(
    program: &str,
    args: &[&str],
    work_dir: &Path,
) -> anyhow::Result<Output> {
    match (program, args) {
        ("mv", [from, to]) => {
            fs::rename(work_dir.join(from), work_dir.join(to))?;
            Ok(successful_output(Vec::new()))
        }
        ("rm", ["-rf", path]) => {
            let path = work_dir.join(path);
            if path.exists() {
                fs::remove_dir_all(path)?;
            }
            Ok(successful_output(Vec::new()))
        }
        ("cat", paths) => {
            let mut stdout = Vec::new();
            for path in paths {
                stdout.extend(fs::read(work_dir.join(path))?);
            }
            Ok(successful_output(stdout))
        }
        ("grep", [pattern, path]) => {
            let contents = fs::read_to_string(work_dir.join(path))?;
            let stdout = contents.lines().filter(|line| line.contains(pattern)).fold(
                String::new(),
                |mut output, line| {
                    writeln!(output, "{line}").unwrap();
                    output
                },
            );
            Ok(successful_output(stdout.into_bytes()))
        }
        ("diff", ["-s", left, right]) => {
            anyhow::ensure!(
                files_equal(&work_dir.join(left), &work_dir.join(right))?,
                "Files {left} and {right} differ"
            );
            Ok(successful_output(
                format!("Files {left} and {right} are identical\n").into_bytes(),
            ))
        }
        ("diff", ["-r", "-s", left, right]) => {
            let left_root = work_dir.join(left);
            let right_root = work_dir.join(right);
            let left_files = relative_files(&left_root)?;
            anyhow::ensure!(
                left_files == relative_files(&right_root)?,
                "Directories {left} and {right} differ"
            );

            let mut stdout = String::new();
            for relative_path in left_files {
                anyhow::ensure!(
                    files_equal(
                        &left_root.join(&relative_path),
                        &right_root.join(&relative_path)
                    )?,
                    "Files differ: {}",
                    relative_path.display()
                );
                let relative_path = relative_path.to_string_lossy().replace('\\', "/");
                writeln!(
                    stdout,
                    "Files {left}/{relative_path} and {right}/{relative_path} are identical"
                )?;
            }
            Ok(successful_output(stdout.into_bytes()))
        }
        _ => Ok(Command::new(program)
            .args(args)
            .current_dir(work_dir)
            .output()?),
    }
}

fn normalize_test_output(output: &str) -> String {
    let output = output.replace("\r\n", "\n");
    #[cfg(windows)]
    let output = output
        .replace("\\\\", "/")
        .replace('\\', "/")
        .replace("move.exe", "move")
        .replace(
            "The system cannot find the path specified. (os error 3)",
            "No such file or directory (os error 2)",
        );
    output
}

/// Run the `args_path` batch file with`cli_binary`
pub fn run_one(
    args_path: &Path,
    cli_binary: &Path,
    use_temp_dir: bool,
    track_cov: bool,
) -> anyhow::Result<Option<ExecCoverageMapWithModules>> {
    let args_file = io::BufReader::new(File::open(args_path)?).lines();
    let cli_binary_path = cli_binary.canonicalize()?;

    // path where we will run the binary
    let exe_dir = args_path.parent().unwrap();
    let temp_dir = if use_temp_dir {
        // symlink everything in the exe_dir into the temp_dir
        let dir = tempdir()?;
        let padded_dir = copy_deps(dir.path(), exe_dir)?;
        simple_copy_dir(&padded_dir, exe_dir)?;
        Some((dir, padded_dir))
    } else {
        None
    };
    let wks_dir = temp_dir.as_ref().map_or(exe_dir, |t| &t.1);

    let storage_dir = wks_dir.join(DEFAULT_STORAGE_DIR);
    let build_output = wks_dir
        .join(DEFAULT_BUILD_DIR)
        .join(CompiledPackageLayout::Root.path());

    // template for preparing a cli command
    let cli_command_template = || {
        let mut command = Command::new(cli_binary_path.clone());
        if let Some(work_dir) = temp_dir.as_ref() {
            command.current_dir(&work_dir.1);
        } else {
            command.current_dir(exe_dir);
        }
        command
    };

    if storage_dir.exists() || build_output.exists() {
        // need to clean before testing
        cli_command_template()
            .arg("sandbox")
            .arg("clean")
            .output()?;
    }
    let mut output = "".to_string();

    // always use the absolute path for the trace file as we may change dirs in the process
    let trace_file = if track_cov {
        Some(wks_dir.canonicalize()?.join(DEFAULT_TRACE_FILE))
    } else {
        None
    };

    // Disable colors in error reporting from the Move compiler
    unsafe { env::set_var(COLOR_MODE_ENV_VAR, "NONE") };
    let mut has_run_command = false;
    for args_line in args_file {
        let args_line = args_line?;

        if let Some(external_cmd) = args_line.strip_prefix('>') {
            let external_cmd = external_cmd.trim_start();
            let mut cmd_iter = external_cmd.split_ascii_whitespace();

            let external_program = cmd_iter.next().expect("empty external command");
            let external_args = cmd_iter.collect::<Vec<_>>();
            let cmd_output = external_command_output(external_program, &external_args, wks_dir)?;

            writeln!(&mut output, "External Command `{}`:", external_cmd)?;
            output += std::str::from_utf8(&cmd_output.stdout)?;
            output += std::str::from_utf8(&cmd_output.stderr)?;

            continue;
        }

        if args_line.starts_with('#') {
            // allow comments in args.txt
            continue;
        }
        let args_iter: Vec<&str> = args_line.split_whitespace().collect();
        if args_iter.is_empty() {
            // allow blank lines in args.txt
            continue;
        }
        has_run_command |= args_iter.starts_with(&["sandbox", "run"]);

        // enable tracing in the VM by setting the env var.
        match &trace_file {
            None => {
                // this check prevents cascading the coverage tracking flag.
                // in particular, if
                //   1. we run with move-cli test <path-to-args-A.txt> --track-cov, and
                //   2. in this <args-A.txt>, there is another command: test <args-B.txt>
                // then, when running <args-B.txt>, coverage will not be tracked nor printed
                unsafe { env::remove_var(MOVE_VM_TRACING_ENV_VAR_NAME) };
            }
            Some(path) => unsafe { env::set_var(MOVE_VM_TRACING_ENV_VAR_NAME, path.as_os_str()) },
        }

        let cmd_output = cli_command_template().args(args_iter).output()?;
        writeln!(&mut output, "Command `{}`:", args_line)?;
        output += std::str::from_utf8(&cmd_output.stdout)?;
        output += std::str::from_utf8(&cmd_output.stderr)?;
    }

    // collect coverage information
    let cov_info = match &trace_file {
        None => None,
        Some(_) if !has_run_command => None,
        Some(trace_path) => {
            if trace_path.exists() {
                Some(collect_coverage(trace_path, &build_output)?)
            } else {
                eprintln!(
                    "Trace file {:?} not found: coverage is only available with at least one `run` \
                    command in the args.txt (after a `clean`, if there is one)",
                    trace_path
                );
                None
            }
        }
    };

    // post-test cleanup and cleanup checks
    // check that the test command didn't create a src dir
    let run_move_clean = !read_bool_env_var(NO_MOVE_CLEAN);
    if run_move_clean {
        // run the clean command to ensure that temporary state is cleaned up
        cli_command_template()
            .arg("sandbox")
            .arg("clean")
            .output()?;

        // check that build and storage was deleted
        assert!(
            !storage_dir.exists(),
            "`move clean` failed to eliminate {} directory",
            DEFAULT_STORAGE_DIR
        );
        assert!(
            !build_output.exists(),
            "`move clean` failed to eliminate {} directory",
            DEFAULT_BUILD_DIR
        );

        // clean the trace file as well if it exists
        if let Some(trace_path) = &trace_file
            && trace_path.exists()
        {
            fs::remove_file(trace_path)?;
        }
    }

    // release the temporary workspace explicitly
    if let Some((t, _)) = temp_dir {
        t.close()?;
    }

    // compare output and exp_file
    let update_baseline = read_env_update_baseline();
    let exp_path = args_path.with_extension(EXP_EXT);
    let output = normalize_test_output(&output);
    if update_baseline {
        fs::write(exp_path, &output)?;
        return Ok(cov_info);
    }

    let expected_output =
        normalize_test_output(&fs::read_to_string(exp_path).unwrap_or_else(|_| "".to_string()));
    if expected_output != output {
        let msg = format!(
            "Expected output differs from actual output:\n{}",
            format_diff(expected_output, output)
        );
        anyhow::bail!(add_update_baseline_fix(msg))
    } else {
        Ok(cov_info)
    }
}

pub fn run_all(
    args_path: &Path,
    cli_binary: &Path,
    use_temp_dir: bool,
    track_cov: bool,
) -> anyhow::Result<()> {
    let mut test_total: u64 = 0;
    let mut test_passed: u64 = 0;
    let mut cov_info = ExecCoverageMapWithModules::empty();

    // find `args.txt` and iterate over them
    for entry in find_filenames(&[args_path], |fpath| {
        fpath.file_name().expect("unexpected file entry path") == TEST_ARGS_FILENAME
    })? {
        match run_one(Path::new(&entry), cli_binary, use_temp_dir, track_cov) {
            Ok(cov_opt) => {
                test_passed = test_passed.checked_add(1).unwrap();
                if let Some(cov) = cov_opt {
                    cov_info.merge(cov);
                }
            }
            Err(ex) => eprintln!("Test {} failed with error: {}", entry, ex),
        }
        test_total = test_total.checked_add(1).unwrap();
    }
    println!("{} / {} test(s) passed.", test_passed, test_total);

    // if any test fails, bail
    let test_failed = test_total.checked_sub(test_passed).unwrap();
    if test_failed != 0 {
        anyhow::bail!("{} / {} test(s) failed.", test_failed, test_total)
    }

    // show coverage information if requested
    if track_cov {
        let mut summary_writer: Box<dyn Write> = Box::new(io::stdout());
        for (_, module_summary) in cov_info.into_module_summaries() {
            module_summary.summarize_human(&mut summary_writer, true)?;
        }
    }

    Ok(())
}
