use std::process::Command;

const REPOSITORY_ENVIRONMENT: [&str; 15] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_GRAFT_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_NAMESPACE",
    "GIT_QUARANTINE_PATH",
    "GIT_INTERNAL_SUPER_PREFIX",
];

pub(crate) fn clear_repository_environment(command: &mut Command) {
    for variable in REPOSITORY_ENVIRONMENT {
        command.env_remove(variable);
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::REPOSITORY_ENVIRONMENT;
    use super::clear_repository_environment;

    #[test]
    fn repository_commands_ignore_ambient_repository_selection() {
        let mut command = Command::new("git");
        clear_repository_environment(&mut command);

        for variable in REPOSITORY_ENVIRONMENT {
            assert!(
                command
                    .get_envs()
                    .any(|(name, value)| name == variable && value.is_none())
            );
        }
    }
}
