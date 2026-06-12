//! Workflow engine — orchestrates the scan → resolve → generate pipeline.

use crate::models::{ProjectConfig, Result};
use std::time::Instant;

/// Phases of the workflow pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPhase {
    /// Scanning source files on disk.
    Scanning,
    /// Discovering project structure and modules.
    Discovering,
    /// Analyzing dependencies between modules.
    Analyzing,
    /// Generating CMake configuration files.
    Generating,
    /// Validating generated output.
    Validating,
    /// All phases completed successfully.
    Complete,
}

impl WorkflowPhase {
    /// Total number of phases (excluding Complete).
    const PHASE_COUNT: f32 = 5.0;

    /// Return a human-readable label for the phase.
    pub fn label(&self) -> &'static str {
        match self {
            WorkflowPhase::Scanning => "Scanning source files",
            WorkflowPhase::Discovering => "Discovering modules",
            WorkflowPhase::Analyzing => "Analyzing dependencies",
            WorkflowPhase::Generating => "Generating CMake files",
            WorkflowPhase::Validating => "Validating output",
            WorkflowPhase::Complete => "Complete",
        }
    }

    /// Next phase in the pipeline.
    pub fn next(&self) -> Option<WorkflowPhase> {
        match self {
            WorkflowPhase::Scanning => Some(WorkflowPhase::Discovering),
            WorkflowPhase::Discovering => Some(WorkflowPhase::Analyzing),
            WorkflowPhase::Analyzing => Some(WorkflowPhase::Generating),
            WorkflowPhase::Generating => Some(WorkflowPhase::Validating),
            WorkflowPhase::Validating => Some(WorkflowPhase::Complete),
            WorkflowPhase::Complete => None,
        }
    }
}

/// Result of a full or incremental workflow execution.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    /// Number of logical modules discovered.
    pub modules_found: usize,
    /// Total number of source/header files scanned.
    pub files_scanned: usize,
    /// Number of dependency edges found between modules.
    pub dependencies_found: usize,
    /// Number of CMakeLists.txt files generated.
    pub cmake_files_generated: usize,
    /// Non-fatal errors encountered during execution.
    pub errors: Vec<String>,
    /// Warnings encountered during execution.
    pub warnings: Vec<String>,
    /// Total wall-clock duration of the workflow in milliseconds.
    pub duration_ms: u64,
}

impl WorkflowResult {
    /// Create an empty result, used during incremental builds before merging.
    pub fn empty() -> Self {
        Self {
            modules_found: 0,
            files_scanned: 0,
            dependencies_found: 0,
            cmake_files_generated: 0,
            errors: vec![],
            warnings: vec![],
            duration_ms: 0,
        }
    }

    /// Merge another result into this one (additive counters).
    pub fn merge(&mut self, other: &WorkflowResult) {
        self.modules_found += other.modules_found;
        self.files_scanned += other.files_scanned;
        self.dependencies_found += other.dependencies_found;
        self.cmake_files_generated += other.cmake_files_generated;
        self.errors.extend(other.errors.clone());
        self.warnings.extend(other.warnings.clone());
    }

    /// Returns true if the workflow completed without errors.
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The workflow engine drives the full scan → resolve → generate pipeline.
pub struct WorkflowEngine {
    /// Project configuration used for this run.
    pub config: ProjectConfig,
    /// Current phase of the pipeline.
    current_phase: WorkflowPhase,
    /// Accumulated result that phases append to.
    result: WorkflowResult,
    /// Timestamp when the workflow started.
    started_at: Option<Instant>,
}

impl WorkflowEngine {
    /// Create a new engine bound to the given project configuration.
    pub fn new(config: ProjectConfig) -> Self {
        Self {
            config,
            current_phase: WorkflowPhase::Scanning,
            result: WorkflowResult::empty(),
            started_at: None,
        }
    }

    /// Run the full pipeline from Scanning through Complete.
    ///
    /// Each phase is a hook that can be replaced or extended by the caller
    /// before invoking `execute_full`. The default implementation advances
    /// through phases without performing real work — real logic is injected
    /// via the phase-specific methods.
    pub fn execute_full(&mut self) -> Result<WorkflowResult> {
        self.started_at = Some(Instant::now());

        self.run_scanning()?;
        self.advance_to(WorkflowPhase::Discovering);

        self.run_discovering()?;
        self.advance_to(WorkflowPhase::Analyzing);

        self.run_analyzing()?;
        self.advance_to(WorkflowPhase::Generating);

        self.run_generating()?;
        self.advance_to(WorkflowPhase::Validating);

        self.run_validating()?;
        self.advance_to(WorkflowPhase::Complete);

        if let Some(start) = self.started_at {
            self.result.duration_ms = start.elapsed().as_millis() as u64;
        }

        Ok(self.result.clone())
    }

    /// Run an incremental workflow, processing only `changed_files`.
    ///
    /// Only phases affected by the changed files are re-executed.
    pub fn execute_incremental(&mut self, changed_files: &[std::path::PathBuf]) -> Result<WorkflowResult> {
        self.started_at = Some(Instant::now());

        if changed_files.is_empty() {
            self.current_phase = WorkflowPhase::Complete;
            if let Some(start) = self.started_at {
                self.result.duration_ms = start.elapsed().as_millis() as u64;
            }
            return Ok(self.result.clone());
        }

        // For incremental runs we re-scan the changed files, re-resolve
        // dependencies, and re-generate only affected modules.
        self.current_phase = WorkflowPhase::Scanning;
        self.run_scanning_incremental(changed_files)?;

        self.advance_to(WorkflowPhase::Analyzing);
        self.run_analyzing_incremental(changed_files)?;

        self.advance_to(WorkflowPhase::Generating);
        self.run_generating_incremental(changed_files)?;

        self.advance_to(WorkflowPhase::Complete);

        if let Some(start) = self.started_at {
            self.result.duration_ms = start.elapsed().as_millis() as u64;
        }

        Ok(self.result.clone())
    }

    /// Return workflow progress as a float in [0.0, 1.0].
    pub fn progress(&self) -> f32 {
        let phase_index = match self.current_phase {
            WorkflowPhase::Scanning => 0,
            WorkflowPhase::Discovering => 1,
            WorkflowPhase::Analyzing => 2,
            WorkflowPhase::Generating => 3,
            WorkflowPhase::Validating => 4,
            WorkflowPhase::Complete => 5,
        };
        phase_index as f32 / WorkflowPhase::PHASE_COUNT
    }

    /// Return the current workflow phase.
    pub fn current_phase(&self) -> &WorkflowPhase {
        &self.current_phase
    }

    /// Return a reference to the accumulated result.
    pub fn result(&self) -> &WorkflowResult {
        &self.result
    }

    // ── phase transitions ──────────────────────────────────────────

    fn advance_to(&mut self, phase: WorkflowPhase) {
        self.current_phase = phase;
    }

    // ── full-run phase hooks (overrideable by external callers) ────

    fn run_scanning(&mut self) -> Result<()> {
        // Stub: real scanning is performed by the scanner module.
        // Callers should replace or wrap this method.
        Ok(())
    }

    fn run_discovering(&mut self) -> Result<()> {
        // Stub: module discovery groups scanned files into CMakeModules.
        Ok(())
    }

    fn run_analyzing(&mut self) -> Result<()> {
        // Stub: dependency analysis builds the DependencyGraph.
        Ok(())
    }

    fn run_generating(&mut self) -> Result<()> {
        // Stub: CMake generation uses tera templates.
        Ok(())
    }

    fn run_validating(&mut self) -> Result<()> {
        // Stub: validation checks generated output for correctness.
        Ok(())
    }

    // ── incremental-run phase hooks ─────────────────────────────────

    fn run_scanning_incremental(&mut self, _changed: &[std::path::PathBuf]) -> Result<()> {
        Ok(())
    }

    fn run_analyzing_incremental(&mut self, _changed: &[std::path::PathBuf]) -> Result<()> {
        Ok(())
    }

    fn run_generating_incremental(&mut self, _changed: &[std::path::PathBuf]) -> Result<()> {
        Ok(())
    }
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BuildBackend, Compiler, TargetArch};
    use std::path::PathBuf;

    fn make_config() -> ProjectConfig {
        ProjectConfig {
            name: "test-proj".into(),
            version: "0.1.0".into(),
            root: PathBuf::from("/tmp/test-proj"),
            language: "CXX".into(),
            c_standard: "11".into(),
            cpp_standard: "17".into(),
            target_arch: TargetArch::X86_64,
            compiler: Compiler::GCC,
            build_backend: BuildBackend::Ninja,
            cmake_min_version: "3.16".into(),
            exclude_dirs: vec!["build".into()],
            output_dir: PathBuf::from("build"),
            enable_watch: false,
            modules: vec![],
            generated_at: String::new(),
            cmake_presets: None,
            toolchain_files: vec![],
            toolchain: None,
        }
    }

    #[test]
    fn phase_progression() {
        let mut phase = WorkflowPhase::Scanning;
        assert_eq!(phase, WorkflowPhase::Scanning);
        phase = phase.next().unwrap();
        assert_eq!(phase, WorkflowPhase::Discovering);
        phase = phase.next().unwrap();
        assert_eq!(phase, WorkflowPhase::Analyzing);
        phase = phase.next().unwrap();
        assert_eq!(phase, WorkflowPhase::Generating);
        phase = phase.next().unwrap();
        assert_eq!(phase, WorkflowPhase::Validating);
        phase = phase.next().unwrap();
        assert_eq!(phase, WorkflowPhase::Complete);
        assert!(phase.next().is_none());
    }

    #[test]
    fn phase_labels() {
        assert_eq!(WorkflowPhase::Scanning.label(), "Scanning source files");
        assert_eq!(WorkflowPhase::Complete.label(), "Complete");
    }

    #[test]
    fn progress_values() {
        let engine = WorkflowEngine::new(make_config());
        assert!((engine.progress() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn execute_full_runs_pipeline() {
        let mut engine = WorkflowEngine::new(make_config());
        let result = engine.execute_full().unwrap();
        assert!(result.is_success());
        // No-op stub phases may complete in <1ms, so duration_ms can be 0.
        assert_eq!(*engine.current_phase(), WorkflowPhase::Complete);
    }

    #[test]
    fn execute_incremental_empty_changes() {
        let mut engine = WorkflowEngine::new(make_config());
        let result = engine.execute_incremental(&[]).unwrap();
        assert!(result.is_success());
        assert_eq!(*engine.current_phase(), WorkflowPhase::Complete);
    }

    #[test]
    fn execute_incremental_with_changes() {
        let mut engine = WorkflowEngine::new(make_config());
        let changed = vec![PathBuf::from("src/main.cpp")];
        let result = engine.execute_incremental(&changed).unwrap();
        assert!(result.is_success());
        assert_eq!(*engine.current_phase(), WorkflowPhase::Complete);
    }

    #[test]
    fn result_merge() {
        let mut a = WorkflowResult::empty();
        a.files_scanned = 10;
        let b = WorkflowResult {
            modules_found: 2,
            files_scanned: 5,
            dependencies_found: 1,
            cmake_files_generated: 2,
            errors: vec![],
            warnings: vec!["unused var".into()],
            duration_ms: 100,
        };
        a.merge(&b);
        assert_eq!(a.modules_found, 2);
        assert_eq!(a.files_scanned, 15);
        assert_eq!(a.dependencies_found, 1);
        assert_eq!(a.cmake_files_generated, 2);
        assert_eq!(a.warnings.len(), 1);
    }
}
