use std::os::unix::fs::symlink;

use super::*;

fn edit(paths: &[&str]) -> PermissionCall {
    PermissionCall {
        tool: PermissionTool::Edit,
        target: String::new(),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
    }
}

#[test]
fn resolved_edit_paths_become_absolute_under_the_workdir_even_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().canonicalize().unwrap();

    let call = resolved(&workdir, &edit(&["src/new.rs", "a/../b.rs"]));

    assert_eq!(call.paths[0], workdir.join("src/new.rs").to_string_lossy());
    assert_eq!(call.paths[1], workdir.join("a/../b.rs").to_string_lossy());
}

#[test]
fn resolved_edit_path_follows_a_link_out_of_the_workdir() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let workdir = root.join("work");
    let outside = root.join("outside");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, workdir.join("escape")).unwrap();

    let call = resolved(&workdir, &edit(&["escape/secret.txt"]));

    assert_eq!(
        call.paths,
        vec![outside.join("secret.txt").to_string_lossy()]
    );
}

#[test]
fn resolved_leaves_other_tools_alone() {
    let call = PermissionCall {
        tool: PermissionTool::Shell,
        target: "ls".to_owned(),
        paths: Vec::new(),
    };

    assert_eq!(resolved(Path::new("/work"), &call), call);
}
