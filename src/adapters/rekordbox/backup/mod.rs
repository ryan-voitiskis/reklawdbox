mod error;
mod interactive;
mod output;
mod process_group;
mod script;
mod supervisor;

#[cfg(test)]
mod tests;

pub(crate) use interactive::execute_embedded_interactive;
pub(crate) use supervisor::run_pre_op_backup;

#[cfg(test)]
pub(crate) use script::write_embedded_script_for_test;
#[cfg(test)]
pub(crate) use supervisor::{
    BackupStatus, execute_script_with_timeout_and_activity_for_test,
    execute_script_with_timeout_for_test, override_pre_op_backup_timeout_for_test,
};
