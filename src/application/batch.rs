//! Transport-independent terminal accounting for bounded batch workflows.

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BatchOutcome {
    pub(crate) command: &'static str,
    pub(crate) operation_failures: u32,
    pub(crate) worker_join_failures: u32,
    pub(crate) writer_failures: u32,
    pub(crate) incomplete: usize,
    pub(crate) user_cancelled: bool,
    pub(crate) error_summaries: Vec<String>,
}

impl BatchOutcome {
    pub(crate) fn finish(self) -> Result<(), BatchFailure> {
        if self.operation_failures == 0
            && self.worker_join_failures == 0
            && self.writer_failures == 0
            && self.incomplete == 0
            && !self.user_cancelled
        {
            Ok(())
        } else {
            Err(BatchFailure {
                command: self.command,
                track_or_provider_failures: self.operation_failures,
                worker_join_failures: self.worker_join_failures,
                writer_failures: self.writer_failures,
                incomplete: self.incomplete,
                user_cancelled: self.user_cancelled,
                error_summaries: self.error_summaries,
            })
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BatchFailure {
    pub(crate) command: &'static str,
    pub(crate) track_or_provider_failures: u32,
    pub(crate) worker_join_failures: u32,
    pub(crate) writer_failures: u32,
    pub(crate) incomplete: usize,
    pub(crate) user_cancelled: bool,
    pub(crate) error_summaries: Vec<String>,
}

impl std::fmt::Display for BatchFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} batch failed: {} track/provider failures, {} task join failures, {} cache write failures, {} incomplete",
            self.command,
            self.track_or_provider_failures,
            self.worker_join_failures,
            self.writer_failures,
            self.incomplete,
        )?;
        if self.user_cancelled {
            write!(formatter, ", cancelled by user")?;
        }
        if !self.error_summaries.is_empty() {
            write!(formatter, ": {}", self.error_summaries.join("; "))?;
        }
        Ok(())
    }
}

impl std::error::Error for BatchFailure {}

pub(crate) fn task_join_error_summary(task: &str, error: &tokio::task::JoinError) -> String {
    if error.is_cancelled() {
        format!("{task} was cancelled")
    } else if error.is_panic() {
        format!("{task} panicked")
    } else {
        format!("{task} failed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome() -> BatchOutcome {
        BatchOutcome {
            command: "test",
            operation_failures: 0,
            worker_join_failures: 0,
            writer_failures: 0,
            incomplete: 0,
            user_cancelled: false,
            error_summaries: Vec::new(),
        }
    }

    #[test]
    fn complete_batch_succeeds() {
        assert_eq!(outcome().finish(), Ok(()));
    }

    #[test]
    fn every_terminal_failure_category_fails_the_batch() {
        let mut cases = Vec::new();

        let mut operation = outcome();
        operation.operation_failures = 1;
        cases.push(operation);

        let mut join = outcome();
        join.worker_join_failures = 1;
        cases.push(join);

        let mut writer = outcome();
        writer.writer_failures = 1;
        cases.push(writer);

        let mut incomplete = outcome();
        incomplete.incomplete = 1;
        cases.push(incomplete);

        let mut cancelled = outcome();
        cancelled.user_cancelled = true;
        cases.push(cancelled);

        for case in cases {
            assert!(case.finish().is_err());
        }
    }

    #[test]
    fn display_preserves_terminal_accounting_vocabulary() {
        let failure = BatchOutcome {
            command: "hydrate",
            operation_failures: 1,
            worker_join_failures: 2,
            writer_failures: 3,
            incomplete: 4,
            user_cancelled: true,
            error_summaries: vec!["stable summary".to_string()],
        }
        .finish()
        .expect_err("failure categories should reject the batch");

        assert_eq!(
            failure.to_string(),
            "hydrate batch failed: 1 track/provider failures, 2 task join failures, 3 cache write failures, 4 incomplete, cancelled by user: stable summary"
        );
    }
}
