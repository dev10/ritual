use crate::cpp_code_generator;
use crate::cpp_code_generator::generate_cpp_type_size_requester;
use crate::processor::ProcessorData;
use crate::rust_code_generator;
use crate::rust_info::RustItem;
use crate::versions;
use itertools::Itertools;
use ritual_common::errors::Result;
use ritual_common::file_utils::{
    copy_file, copy_recursively, create_dir, create_dir_all, create_file, diff_paths, path_to_str,
    read_dir, remove_dir_all, remove_file, repo_dir_path, save_json, save_toml_table,
};
use ritual_common::toml;
use ritual_common::utils::{run_command, MapIfOk};
use ritual_common::BuildScriptData;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Merges `a` and `b` recursively. `b` take precedence over `a`.
fn recursive_merge_toml(a: toml::Value, b: toml::Value) -> toml::Value {
    if a.same_type(&b) {
        if let toml::Value::Array(mut a_array) = a {
            if let toml::Value::Array(mut b_array) = b {
                a_array.append(&mut b_array);
                toml::Value::Array(a_array)
            } else {
                unreachable!()
            }
        } else if let toml::Value::Table(mut a_table) = a {
            if let toml::Value::Table(b_table) = b {
                for (key, value) in b_table {
                    if let Some(old_value) = a_table.remove(&key) {
                        a_table.insert(key, recursive_merge_toml(old_value, value));
                    } else {
                        a_table.insert(key, value);
                    }
                }
                toml::Value::Table(a_table)
            } else {
                unreachable!()
            }
        } else {
            b
        }
    } else {
        b
    }
}

/// Generates `Cargo.toml` file and skeleton of the crate.
/// If a crate template was supplied, files from it are
/// copied to the output location.
fn generate_crate_template(data: &mut ProcessorData<'_>) -> Result<()> {
    let output_path = data
        .workspace
        .crate_path(data.config.crate_properties().name())?;

    let template_build_rs_path =
        data.config
            .crate_template_path()
            .as_ref()
            .and_then(|crate_template_path| {
                let template_build_rs_path = crate_template_path.join("build.rs");
                if template_build_rs_path.exists() {
                    Some(template_build_rs_path)
                } else {
                    None
                }
            });
    let output_build_rs_path = output_path.join("build.rs");
    if let Some(template_build_rs_path) = &template_build_rs_path {
        copy_file(template_build_rs_path, output_build_rs_path)?;
    } else {
        let mut build_rs_file = create_file(&output_build_rs_path)?;
        write!(
            build_rs_file,
            "{}",
            include_str!("../templates/crate/build.rs")
        )?;
    }
    let cargo_toml_data = {
        let package = toml::value::Value::Table({
            let mut table = toml::value::Table::new();
            table.insert(
                "name".to_string(),
                toml::Value::String(data.config.crate_properties().name().into()),
            );
            table.insert(
                "version".to_string(),
                toml::Value::String(data.config.crate_properties().version().into()),
            );
            table.insert(
                "build".to_string(),
                toml::Value::String("build.rs".to_string()),
            );
            table.insert(
                "edition".to_string(),
                toml::Value::String("2018".to_string()),
            );
            table
        });
        let dep_value = |version: &str, local_path: Option<PathBuf>| -> Result<toml::Value> {
            Ok(
                if local_path.is_none() || !data.workspace.config().write_dependencies_local_paths {
                    toml::Value::String(version.to_string())
                } else {
                    let path = diff_paths(&local_path.expect("checked above"), &output_path)?;

                    toml::Value::Table({
                        let mut value = toml::value::Table::new();
                        value.insert(
                            "version".to_string(),
                            toml::Value::String(version.to_string()),
                        );
                        value.insert(
                            "path".to_string(),
                            toml::Value::String(path_to_str(&path)?.to_string()),
                        );
                        value
                    })
                },
            )
        };
        let dependencies = toml::Value::Table({
            let mut table = toml::value::Table::new();
            if !data
                .config
                .crate_properties()
                .should_remove_default_dependencies()
            {
                table.insert(
                    "cpp_utils".to_string(),
                    dep_value(
                        versions::CPP_UTILS_VERSION,
                        if data.workspace.config().write_dependencies_local_paths {
                            Some(repo_dir_path("cpp_utils")?)
                        } else {
                            None
                        },
                    )?,
                );
                for dep in data.dep_databases {
                    table.insert(
                        dep.crate_name().to_string(),
                        dep_value(
                            &dep.crate_version(),
                            Some(data.workspace.crate_path(dep.crate_name())?),
                        )?,
                    );
                }
            }
            for dep in data.config.crate_properties().dependencies() {
                table.insert(
                    dep.name().to_string(),
                    dep_value(dep.version(), dep.local_path().map(PathBuf::from))?,
                );
            }
            table
        });
        let build_dependencies = toml::Value::Table({
            let mut table = toml::value::Table::new();
            if !data
                .config
                .crate_properties()
                .should_remove_default_build_dependencies()
            {
                table.insert(
                    "ritual_build".to_string(),
                    dep_value(
                        versions::RITUAL_BUILD_VERSION,
                        if data.workspace.config().write_dependencies_local_paths {
                            Some(repo_dir_path("ritual_build")?)
                        } else {
                            None
                        },
                    )?,
                );
            }
            for dep in data.config.crate_properties().build_dependencies() {
                table.insert(
                    dep.name().to_string(),
                    dep_value(dep.version(), dep.local_path().map(PathBuf::from))?,
                );
            }
            table
        });
        let mut table = toml::value::Table::new();
        table.insert("package".to_string(), package);
        table.insert("dependencies".to_string(), dependencies);
        table.insert("build-dependencies".to_string(), build_dependencies);
        recursive_merge_toml(
            toml::Value::Table(table),
            toml::Value::Table(data.config.crate_properties().custom_fields().clone()),
        )
    };
    save_toml_table(output_path.join("Cargo.toml"), &cargo_toml_data)?;

    if let Some(template_path) = &data.config.crate_template_path() {
        for item in read_dir(template_path)? {
            let item = item?;
            let target = output_path.join(item.file_name());
            copy_recursively(&item.path(), &target)?;
        }
    }
    if !output_path.join("src").exists() {
        create_dir_all(output_path.join("src"))?;
    }
    Ok(())
}

/// Generates main files and directories of the library.
fn generate_c_lib_template(
    lib_name: &str,
    lib_path: &Path,
    global_header_name: &str,
    include_directives: &[PathBuf],
) -> Result<()> {
    let name_upper = lib_name.to_uppercase();
    let cmakelists_path = lib_path.join("CMakeLists.txt");
    let mut cmakelists_file = create_file(&cmakelists_path)?;

    write!(
        cmakelists_file,
        include_str!("../templates/c_lib/CMakeLists.txt"),
        lib_name_lowercase = lib_name,
        lib_name_uppercase = name_upper
    )?;

    let include_directives_code = include_directives
        .map_if_ok(|d| -> Result<_> { Ok(format!("#include \"{}\"", path_to_str(d)?)) })?
        .join("\n");

    let global_header_path = lib_path.join(&global_header_name);
    let mut global_header_file = create_file(&global_header_path)?;
    write!(
        global_header_file,
        include_str!("../templates/c_lib/global.h"),
        include_directives_code = include_directives_code
    )?;
    Ok(())
}

pub fn run(data: &mut ProcessorData<'_>) -> Result<()> {
    let crate_name = data.config.crate_properties().name();
    let output_path = data.workspace.crate_path(crate_name)?;

    for item in read_dir(&output_path)? {
        let path = item?.path();
        if path
            == data
                .workspace
                .database_path(data.config.crate_properties().name())
        {
            continue;
        }
        if path.is_dir() {
            remove_dir_all(&path)?;
        } else {
            remove_file(&path)?;
        }
    }

    generate_crate_template(data)?;
    data.workspace.update_cargo_toml()?;

    let c_lib_path = output_path.join("c_lib");
    if !c_lib_path.exists() {
        create_dir(&c_lib_path)?;
    }
    let c_lib_name = format!("{}_c", data.config.crate_properties().name());
    let global_header_name = format!("{}_global.h", c_lib_name);
    generate_c_lib_template(
        &c_lib_name,
        &c_lib_path,
        &global_header_name,
        data.config.include_directives(),
    )?;

    let used_ffi_functions = data
        .current_database
        .rust_items()
        .iter()
        .filter_map(|item| {
            if let RustItem::FfiFunction(func) = &item.item {
                Some(func.path.last())
            } else {
                None
            }
        })
        .collect::<HashSet<&str>>();

    cpp_code_generator::generate_cpp_file(
        &data
            .current_database
            .ffi_items()
            .iter()
            .filter(|item| {
                !item.item.is_function()
                    || used_ffi_functions.contains(item.path().last().name.as_str())
            })
            .collect_vec(),
        data.current_database.environments(),
        &c_lib_path.join("file1.cpp"),
        &global_header_name,
        data.current_database.crate_name(),
    )?;

    let file = create_file(c_lib_path.join("sized_types.cxx"))?;
    generate_cpp_type_size_requester(
        data.current_database.rust_database(),
        data.config.include_directives(),
        file,
    )?;

    rust_code_generator::generate(
        data.config.crate_properties().name(),
        &data.current_database,
        data.current_database.rust_database(),
        &output_path.join("src"),
        data.config.crate_template_path().map(|s| s.join("src")),
    )?;

    // -p shouldn't be needed, it's a workaround for this bug on Windows:
    // https://github.com/rust-lang/rustfmt/issues/2694
    run_command(
        Command::new("cargo")
            .arg("fmt")
            .arg(format!("-p{}", crate_name))
            .current_dir(&output_path),
    )?;
    run_command(
        Command::new("rustfmt")
            .arg("src/ffi.in.rs")
            .current_dir(&output_path),
    )?;

    save_json(
        output_path.join("build_script_data.json"),
        &BuildScriptData {
            cpp_build_config: data.config.cpp_build_config().clone(),
            cpp_wrapper_lib_name: c_lib_name,
            known_targets: data.current_database.environments().to_vec(),
        },
        None,
    )?;
    Ok(())
}
