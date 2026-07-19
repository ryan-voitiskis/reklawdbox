use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::super::activation::ManagedEnvironmentPaths;
use super::super::contract::ESSENTIA_IMPORT_CHECK_SCRIPT;
use super::super::process::{CommandRequest, CommandResult, CommandRunner, ProcessError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedCall {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) timeout: Duration,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FakeConfig {
    pub(super) python314: bool,
    pub(super) python3: bool,
    pub(super) venv: bool,
    pub(super) pip: bool,
    pub(super) pip_wheel_unavailable: bool,
    pub(super) direct_probe: bool,
    pub(super) stable_probe: bool,
    pub(super) sabotage_stable_before_failure: bool,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            python314: true,
            python3: true,
            venv: true,
            pip: true,
            pip_wheel_unavailable: false,
            direct_probe: true,
            stable_probe: true,
            sabotage_stable_before_failure: false,
        }
    }
}

pub(super) struct FakeCommandRunner {
    paths: ManagedEnvironmentPaths,
    config: FakeConfig,
    calls: Mutex<Vec<RecordedCall>>,
    new_generation: Mutex<Option<PathBuf>>,
}

impl FakeCommandRunner {
    pub(super) fn new(paths: ManagedEnvironmentPaths, config: FakeConfig) -> Self {
        Self {
            paths,
            config,
            calls: Mutex::new(Vec::new()),
            new_generation: Mutex::new(None),
        }
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeCommandRunner {
    fn run(&self, request: CommandRequest<'_>) -> Result<CommandResult, ProcessError> {
        let program = request.program;
        let args = request.args;
        self.calls.lock().unwrap().push(RecordedCall {
            program: program.to_string(),
            args: args.to_vec(),
            timeout: request.timeout,
        });
        if args == ["-c", ESSENTIA_IMPORT_CHECK_SCRIPT] {
            let generation = self.new_generation.lock().unwrap().clone();
            let is_direct = generation
                .as_ref()
                .is_some_and(|generation| Path::new(program) == generation.join("bin/python"));
            let is_stable = generation.as_ref().is_some_and(|generation| {
                Path::new(program) == self.paths.stable.join("bin/python")
                    && fs::read_link(&self.paths.stable)
                        .ok()
                        .map(|target| {
                            if target.is_absolute() {
                                target
                            } else {
                                self.paths.stable.parent().unwrap().join(target)
                            }
                        })
                        .is_some_and(|target| target == generation.as_path())
            });
            if !is_direct && !is_stable {
                return Ok(CommandResult {
                    success: false,
                    stdout: Vec::new(),
                    stderr: b"scripted runtime unavailable".to_vec(),
                });
            }
            let manifest_matches =
                (is_direct && self.config.direct_probe) || (is_stable && self.config.stable_probe);
            if is_stable && !self.config.stable_probe && self.config.sabotage_stable_before_failure
            {
                fs::remove_file(&self.paths.stable).unwrap();
                fs::create_dir(&self.paths.stable).unwrap();
            }
            return Ok(CommandResult {
                success: true,
                stdout: probe_json(manifest_matches),
                stderr: Vec::new(),
            });
        }
        let success = if args.get(1).is_some_and(|arg| arg == "venv") {
            let generation = PathBuf::from(args.last().unwrap());
            fs::create_dir_all(generation.join("bin")).unwrap();
            fs::write(generation.join("partial-build"), b"incomplete").unwrap();
            if self.config.venv {
                fs::write(generation.join("bin/python"), b"fake python").unwrap();
                *self.new_generation.lock().unwrap() = Some(generation);
            }
            self.config.venv
        } else if args.get(1).is_some_and(|arg| arg == "pip") {
            self.config.pip
        } else if program == "python3.14" {
            self.config.python314
        } else if program == "python3" {
            self.config.python3
        } else {
            false
        };
        Ok(CommandResult {
            success,
            stdout: Vec::new(),
            stderr: if success {
                Vec::new()
            } else if args.get(1).is_some_and(|arg| arg == "pip")
                && self.config.pip_wheel_unavailable
            {
                b"ERROR: No matching distribution found for essentia==2.1b6.dev1438".to_vec()
            } else {
                b"scripted failure".to_vec()
            },
        })
    }
}

fn probe_json(manifest_matches: bool) -> Vec<u8> {
    let numpy = if manifest_matches { "2.5.1" } else { "0.0.0" };
    serde_json::to_vec(&serde_json::json!({
        "python": "3.14.6",
        "implementation": "cpython",
        "essentia": "2.1b6.dev1438",
        "essentia_module": "2.1-beta6-dev",
        "numpy": numpy,
        "pyyaml": "6.0.3",
        "six": "1.17.0"
    }))
    .unwrap()
}
