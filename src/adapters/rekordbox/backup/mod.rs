mod error;
mod interactive;
mod output;
mod process_group;
mod script;
mod supervisor;

#[cfg(test)]
mod tests;

pub(crate) use interactive::execute_embedded_interactive;
// Preserve the established adapter paths even where production callers infer
// the status type or currently enter through `run_pre_op_backup`.
pub(crate) use supervisor::run_pre_op_backup;
#[allow(unused_imports)]
pub(crate) use supervisor::{BackupStatus, execute_embedded_with_env};

#[cfg(test)]
pub(crate) use script::write_embedded_script_for_test;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use supervisor::PreOpBackupTimeoutOverride;
#[cfg(test)]
pub(crate) use supervisor::{
    execute_script_with_timeout_and_activity_for_test, execute_script_with_timeout_for_test,
    override_pre_op_backup_timeout_for_test,
};
