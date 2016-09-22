extern crate serde_json;
extern crate num_cpus;

use std;
use std::fs;
use std::fs::File;
use utils::PathBufPushTweak;
use utils::is_msvc;

use std::path::PathBuf;
use std::process::Command;
use cpp_code_generator::CppCodeGenerator;
use log;
use cpp_parser;
use qt_specific;
use utils;
use cpp_ffi_generator;
use rust_info::{InputCargoTomlData, RustExportInfo};
use rust_code_generator;
use rust_code_generator::{RustCodeGeneratorDependency, RustLinkItem, RustLinkKind};
use rust_generator;
use serializable::LibSpec;
use cpp_ffi_generator::CppAndFfiData;
use qt_doc_parser::QtDocData;
use dependency_info::DependencyInfo;
use std::env;


/// Runs a command, checks that it is successful, and
/// returns its output if requested
fn run_command(command: &mut Command, fetch_stdout: bool) -> String {
  log::info(format!("Executing command: {:?}", command));
  if fetch_stdout {
    match command.output() {
      Ok(output) => {
        match command.status() {
          Ok(status) => {
            if !status.success() {
              panic!("Command failed: {:?} (status: {})", command, status);
            }
          }
          Err(error) => {
            panic!("Execution failed: {}", error);
          }
        }
        String::from_utf8(output.stdout).unwrap()
      }
      Err(error) => {
        panic!("Execution failed: {}", error);
      }
    }
  } else {
    match command.status() {
      Ok(status) => {
        if !status.success() {
          panic!("Command failed: {:?} (status: {})", command, status);
        }
      }
      Err(error) => {
        panic!("Execution failed: {}", error);
      }
    }
    String::new()
  }
}

fn add_env_path_item(env_var_name: &'static str,
                     mut new_paths: Vec<PathBuf>)
                     -> std::ffi::OsString {
  for path in env::split_paths(&env::var(env_var_name).unwrap_or(String::new())) {
    if new_paths.iter().find(|&x| x == &path).is_none() {
      new_paths.push(path);
    }
  }
  env::join_paths(new_paths).unwrap()
}

pub enum BuildProfile {
  Debug,
  Release,
}

pub use rust_code_generator::InvokationMethod;

pub struct BuildEnvironment {
  pub invokation_method: InvokationMethod,
  pub output_dir_path: PathBuf,
  pub source_dir_path: PathBuf,
  pub dependency_paths: Vec<PathBuf>,
  pub num_jobs: Option<i32>,
  pub build_profile: BuildProfile,
}

pub fn run_from_build_script() {
  let mut dependency_paths = Vec::new();
  if env::var("CARGO_MANIFEST_DIR").unwrap() != "/home/ri/rust/rust_qt/repos/qt_gui/../qt_core" {
    for (name, value) in env::vars_os() {
      if let Ok(name) = name.into_string() {
        if name.starts_with("DEP_") && name.ends_with("_CPP_TO_RUST_DATA_PATH") {
          let value = value.into_string().unwrap();
          log::info(format!("Found dependency: {}", &value));
          dependency_paths.push(PathBuf::from(value));
        }
      }
    }
  }
  run(BuildEnvironment {
    invokation_method: InvokationMethod::BuildScript,
    source_dir_path: PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()),
    output_dir_path: PathBuf::from(env::var("OUT_DIR").unwrap()),
    num_jobs: env::var("NUM_JOBS").unwrap().parse().ok(),
    build_profile: match env::var("PROFILE").unwrap().as_ref() {
      "debug" | "test" | "doc" => BuildProfile::Debug,
      "release" | "bench" => BuildProfile::Release,
      a @ _ => panic!("unsupported profile: {}", a),
    },
    dependency_paths: dependency_paths,
  });
}

pub fn run(env: BuildEnvironment) {
  // canonicalize paths
  if !env.source_dir_path.as_path().exists() {
    panic!("Invalid source dir: {}", env.source_dir_path.display());
  }
  if !env.output_dir_path.as_path().exists() {
    fs::create_dir_all(&env.output_dir_path).unwrap();
  }
  let output_dir_path = fs::canonicalize(&env.output_dir_path).unwrap();
  let source_dir_path = fs::canonicalize(&env.source_dir_path).unwrap();

  let lib_spec_path = source_dir_path.with_added("spec.json");

  log::info("Reading lib spec");
  let file = File::open(&lib_spec_path).unwrap();
  let lib_spec: LibSpec = serde_json::from_reader(file).unwrap();

  log::info("Reading input Cargo.toml");
  let input_cargo_toml_data =
    InputCargoTomlData::from_file(&source_dir_path.with_added("Cargo.toml"));
  log::info(format!("C++ library name: {}", lib_spec.cpp.name));

  let is_qt_library = lib_spec.cpp.name.starts_with("Qt5");

  let mut include_dirs = Vec::new();
  let mut cpp_lib_path = None;
  let mut qt_this_lib_headers_dir = None;
  let mut framework_dirs = Vec::new();
  let mut link_items = Vec::new();
  if is_qt_library {

    let qmake_path = "qmake".to_string();
    log::info("Detecting Qt directories...");
    let qt_install_headers_path = PathBuf::from(run_command(Command::new(&qmake_path)
                                                              .arg("-query")
                                                              .arg("QT_INSTALL_HEADERS"),
                                                            true)
      .trim());
    log::info(format!("QT_INSTALL_HEADERS = \"{}\"",
                      qt_install_headers_path.to_str().unwrap()));
    let qt_install_libs_path = PathBuf::from(run_command(Command::new(&qmake_path)
                                                           .arg("-query")
                                                           .arg("QT_INSTALL_LIBS"),
                                                         true)
      .trim());
    log::info(format!("QT_INSTALL_LIBS = \"{}\"",
                      qt_install_libs_path.to_str().unwrap()));
    cpp_lib_path = Some(qt_install_libs_path.clone());
    include_dirs.push(qt_install_headers_path.clone());

    if lib_spec.cpp.name.starts_with("Qt5") {
      let dir = qt_install_headers_path.with_added(format!("Qt{}", &lib_spec.cpp.name[3..]));
      if dir.exists() {
        qt_this_lib_headers_dir = Some(dir.clone());
        include_dirs.push(dir);
      } else {
        let dir2 = qt_install_libs_path.with_added(format!("Qt{}.framework/Headers", &lib_spec.cpp.name[3..]));
        if dir2.exists() {
          qt_this_lib_headers_dir = Some(dir2.clone());
          include_dirs.push(dir2);
          framework_dirs.push(qt_install_libs_path.clone());
          link_items.push(RustLinkItem {
            name: format!("Qt{}", &lib_spec.cpp.name[3..]),
            kind: RustLinkKind::Framework,
          });
        } else {
          log::warning(format!("extra header dir not found (tried: {}, {})",
                               dir.display(),
                               dir2.display()));
        }
      }
    }
  }
  if framework_dirs.is_empty() {
    link_items.push(RustLinkItem {
      name: lib_spec.cpp.name.clone(),
      kind: RustLinkKind::SharedLibrary,
    });
    for name in lib_spec.cpp.extra_libs.as_ref().unwrap_or(&Vec::new()) {
      if is_msvc() && name == "GL" {
        // msvc doesn't need to link to GL
        // TODO: allow platform-specific link items in manifest
        continue;
      }
      link_items.push(RustLinkItem {
        name: name.clone(),
        kind: RustLinkKind::SharedLibrary,
      });
    }
  }
  let qt_doc_data = if is_qt_library {
    // TODO: use env only in build script, switch to cmd arg in cli
    let env_var_name = format!("{}_DOC_DATA", lib_spec.cpp.name.to_uppercase());
    match std::env::var(&env_var_name) {
      Ok(env_var_value) => {
        log::info(format!("Loading Qt doc data"));
        match QtDocData::new(&PathBuf::from(&env_var_value)) {
          Ok(r) => {
            log::info(format!("Loaded Qt doc data from {}", &env_var_value));
            Some(r)
          }
          Err(msg) => {
            log::warning(format!("Failed to load Qt doc data: {}", msg));
            None
          }
        }
      }
      Err(_) => {
        log::warning(format!("Building without Qt doc data (no env var: {})",
                             env_var_name));
        None
      }
    }
  } else {
    None
  };
  if env.dependency_paths.len() > 0 {
    log::info("Loading dependencies");
  }
  let dependencies: Vec<_> = env.dependency_paths
    .iter()
    .map(|path| DependencyInfo::load(&fs::canonicalize(path).unwrap()))
    .collect();

  let c_lib_parent_path = output_dir_path.with_added("c_lib");
  let c_lib_install_path = c_lib_parent_path.with_added("install");
  let c_lib_lib_path = c_lib_install_path.with_added("lib");
  let num_jobs = env.num_jobs.unwrap_or_else(|| num_cpus::get() as i32);
  let mut dependency_cpp_types = Vec::new();
  for dep in &dependencies {
    dependency_cpp_types.extend_from_slice(&dep.cpp_data.types);
  }
  let c_lib_is_shared = is_msvc();
  if output_dir_path.with_added("skip_processing").as_path().exists() {
    log::info("Processing skipped!");
  } else {
    let parse_result_cache_file_path = output_dir_path.with_added("cpp_data.json");
    let parse_result = if parse_result_cache_file_path.as_path().is_file() {
      log::info(format!("C++ data is loaded from file: {}",
                        parse_result_cache_file_path.to_str().unwrap()));
      let file = File::open(&parse_result_cache_file_path).unwrap();
      serde_json::from_reader(file).unwrap()
    } else {
      log::info("Parsing C++ headers.");
      let mut parse_result =
        cpp_parser::run(cpp_parser::CppParserConfig {
                          include_dirs: include_dirs.clone(),
                          framework_dirs: framework_dirs.clone(),
                          header_name: lib_spec.cpp.include_file.clone(),
                          target_include_dir: qt_this_lib_headers_dir.clone(),
                          tmp_cpp_path: output_dir_path.with_added("1.cpp"),
                          name_blacklist: lib_spec.cpp.name_blacklist.clone().unwrap_or(Vec::new()),
                        },
                        &dependency_cpp_types);
      if is_qt_library {
        qt_specific::fix_header_names(&mut parse_result, &qt_this_lib_headers_dir.unwrap());
      }
      log::info("Post-processing parse result.");
      parse_result.post_process(&dependencies.iter().map(|x| &x.cpp_data).collect());

      let mut file = File::create(&parse_result_cache_file_path).unwrap();
      serde_json::to_writer(&mut file, &parse_result).unwrap();
      log::info(format!("Header parse result is saved to file: {}",
                        parse_result_cache_file_path.to_str().unwrap()));
      parse_result
    };

    let c_lib_name = format!("{}_c", &input_cargo_toml_data.name);
    let c_lib_path = c_lib_parent_path.with_added("source");
    let c_lib_tmp_path = c_lib_parent_path.with_added("source.new");
    if c_lib_tmp_path.as_path().exists() {
      fs::remove_dir_all(&c_lib_tmp_path).unwrap();
    }
    fs::create_dir_all(&c_lib_tmp_path).unwrap();
    log::info(format!("Generating C wrapper library ({}).", c_lib_name));

    let cpp_ffi_headers = cpp_ffi_generator::run(&parse_result, lib_spec.cpp.clone());

    let mut cpp_libs = Vec::new();
    if c_lib_is_shared {

      for spec in dependencies.iter()
        .map(|dep| &dep.rust_export_info.lib_spec)
        .chain(std::iter::once(&lib_spec)) {
        cpp_libs.push(spec.cpp.name.clone());
        if let Some(ref extra_libs) = spec.cpp.extra_libs {
          for name in extra_libs {
            if is_msvc() && name == "GL" {
              continue;
            }
            cpp_libs.push(name.clone());
          }
        }
      }
    }
    let code_gen = CppCodeGenerator::new(c_lib_name.clone(),
                                         c_lib_tmp_path.clone(),
                                         c_lib_is_shared,
                                         cpp_libs);
    code_gen.generate_template_files(&lib_spec.cpp.include_file,
                                     &include_dirs.iter()
                                       .map(|x| x.to_str().unwrap().to_string())
                                       .collect(),
                                     &framework_dirs.iter()
                                       .map(|x| x.to_str().unwrap().to_string())
                                       .collect());
    code_gen.generate_files(&cpp_ffi_headers);

    utils::move_files(&c_lib_tmp_path, &c_lib_path).unwrap();

    log::info(format!("Building C wrapper library."));
    let c_lib_build_path = c_lib_parent_path.with_added("build");
    fs::create_dir_all(&c_lib_build_path).unwrap();
    fs::create_dir_all(&c_lib_install_path).unwrap();
    let mut cmake_command = Command::new("cmake");
    fn path_without_long_path(pathbuf: &PathBuf) -> &str {
      let path = pathbuf.to_str().unwrap();
      if path.starts_with(r"\\?\") {
        let result = &path[4..];
        if result.len() > 255 {
          panic!("This path can't be longer than 255 symbols: {}", result);
        }
        result
      } else {
        path
      }
    }
    cmake_command.arg(&path_without_long_path(&c_lib_path))
      .arg(format!("-DCMAKE_INSTALL_PREFIX={}",
                   path_without_long_path(&c_lib_install_path)))
      .current_dir(path_without_long_path(&c_lib_build_path));
    if is_msvc() {
      cmake_command.arg("-G").arg("NMake Makefiles");
      // Rust always links to release version of MSVC runtime, so
      // link will fail if C library is built in debug mode
      cmake_command.arg("-DCMAKE_BUILD_TYPE=Release");
    }
    // TODO: enable release mode on other platforms if cargo is in release mode
    // (maybe build C library in both debug and release in separate folders)
    run_command(&mut cmake_command, false);

    let make_command_name = if is_msvc() { "nmake" } else { "make" }.to_string();
    let mut make_args = Vec::new();
    if !is_msvc() {
      // nmake doesn't support multiple jobs
      // TODO: allow to use jom
      make_args.push(format!("-j{}", num_jobs));
    }
    make_args.push("install".to_string());
    let mut make_command = Command::new(make_command_name);
    make_command.args(&make_args)
      .current_dir(path_without_long_path(&c_lib_build_path));
    if c_lib_is_shared {
      if let Some(ref cpp_lib_path) = cpp_lib_path {
        for name in &["LIBRARY_PATH", "LD_LIBRARY_PATH", "LIB"] {
          make_command.env(name, add_env_path_item(name, vec![cpp_lib_path.clone()]));
        }
      }
    }
    run_command(&mut make_command, false);

    let crate_new_path = output_dir_path.with_added(format!("{}.new", &input_cargo_toml_data.name));
    if crate_new_path.as_path().exists() {
      fs::remove_dir_all(&crate_new_path).unwrap();
    }
    fs::create_dir_all(&crate_new_path).unwrap();
    let rustfmt_config_path = source_dir_path.with_added("rustfmt.toml");
    let rust_config = rust_code_generator::RustCodeGeneratorConfig {
      invokation_method: env.invokation_method.clone(),
      crate_name: input_cargo_toml_data.name.clone(),
      crate_authors: input_cargo_toml_data.authors.clone(),
      crate_version: input_cargo_toml_data.version.clone(),
      output_path: crate_new_path.clone(),
      template_path: source_dir_path.clone(),
      c_lib_name: c_lib_name,
      c_lib_is_shared: c_lib_is_shared,
      link_items: link_items,
      framework_dirs: framework_dirs.iter().map(|x| x.to_str().unwrap().to_string()).collect(),
      rustfmt_config_path: if rustfmt_config_path.as_path().exists() {
        Some(rustfmt_config_path)
      } else {
        None
      },
      dependencies: dependencies.iter()
        .map(|x| {
          RustCodeGeneratorDependency {
            crate_name: x.rust_export_info.crate_name.clone(),
            crate_path: x.path.clone(),
          }
        })
        .collect(),
    };
    log::info(format!("Generating Rust crate ({}).", &input_cargo_toml_data.name));
    let mut dependency_rust_types = Vec::new();
    for dep in &dependencies {
      dependency_rust_types.extend_from_slice(&dep.rust_export_info.rust_types);
    }
    let rust_data = rust_generator::run(CppAndFfiData {
                                          cpp_data: parse_result,
                                          cpp_ffi_headers: cpp_ffi_headers,
                                        },
                                        dependency_rust_types,
                                        rust_generator::RustGeneratorConfig {
                                          crate_name: input_cargo_toml_data.name.clone(),
                                          remove_qt_prefix: is_qt_library,
                                          module_blacklist: lib_spec.rust
                                            .module_blacklist
                                            .clone()
                                            .unwrap_or(Vec::new()),
                                          qt_doc_data: qt_doc_data,
                                        });
    rust_code_generator::run(rust_config, &rust_data);
    {
      let rust_types_path = output_dir_path.with_added("rust_export_info.json");
      let mut file = File::create(&rust_types_path).unwrap();
      serde_json::to_writer(&mut file,
                            &RustExportInfo {
                              crate_name: input_cargo_toml_data.name.clone(),
                              rust_types: rust_data.processed_types,
                              lib_spec: lib_spec.clone(),
                            })
        .unwrap();
      log::info(format!("Rust export info is saved to file: {}",
                        rust_types_path.to_str().unwrap()));
    }

    for item in fs::read_dir(&crate_new_path).unwrap() {
      let item = item.unwrap();
      utils::move_files(&crate_new_path.with_added(item.file_name()),
                        &output_dir_path.with_added(item.file_name()))
        .unwrap();
    }
    fs::remove_dir(&crate_new_path).unwrap();
  }


  match env.invokation_method {
    InvokationMethod::CommandLine => {
      log::info(format!("Compiling Rust crate."));
      let mut lib_dirs = Vec::new();
      if let Some(ref cpp_lib_path) = cpp_lib_path {
        lib_dirs.push(cpp_lib_path.clone());
      }
      if c_lib_is_shared {
        lib_dirs.push(c_lib_lib_path.clone());
      }
      for cargo_cmd in vec!["build", "test", "doc"] {
        let mut command = Command::new("cargo");
        command.arg(cargo_cmd);
        command.arg("--verbose");
        command.arg(format!("-j{}", num_jobs));
        command.current_dir(&output_dir_path);
        // TODO: if env var already exists, add to it instead of overwriting
        if !lib_dirs.is_empty() {
          for name in &["LIBRARY_PATH", "LD_LIBRARY_PATH", "LIB"] {
            command.env(name, add_env_path_item(name, lib_dirs.clone()));
          }
        }
        if !framework_dirs.is_empty() {
          command.env("DYLD_FRAMEWORK_PATH",
                      add_env_path_item("DYLD_FRAMEWORK_PATH", framework_dirs.clone()));
        }
        if is_msvc() && cargo_cmd == "test" {
          // cargo doesn't pass this flag to rustc when it compiles qt_core,
          // so it's compiled with static std and the tests fail with
          // "cannot satisfy dependencies so `std` only shows up once" error.
          command.env("RUSTFLAGS", "-C prefer-dynamic");
        }
        run_command(&mut command, false);
      }
      log::info("Completed successfully.");
    }
    InvokationMethod::BuildScript => {
      println!("cargo:rustc-link-search={}",
               c_lib_lib_path.to_str().unwrap());
      if let Some(ref cpp_lib_path) = cpp_lib_path {
        let lib_path = cpp_lib_path.to_str().unwrap();
        println!("cargo:rustc-link-search=native={}", lib_path);
      }
      println!("cargo:cpp_to_rust_data_path={}",
               output_dir_path.to_str().unwrap());
      for dir in &framework_dirs {
        println!("cargo:rustc-link-search=framework={}",
                 dir.to_str().unwrap());
      }
    }
  }
}