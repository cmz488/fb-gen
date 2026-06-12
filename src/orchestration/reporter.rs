//! Terminal reporter — colored output with indicatif progress bars.

use colored::*;
use indicatif::{ProgressBar, ProgressStyle};

use crate::orchestration::workflow::WorkflowPhase;

/// Renders workflow progress to the terminal using colored text and a
/// single indicatif progress bar.
pub struct Reporter {
    /// Optional progress bar (None when `quiet` is true).
    bar: Option<ProgressBar>,
    /// Suppress all non-error output.
    quiet: bool,
}

impl Reporter {
    /// Create a new reporter.
    ///
    /// When `quiet` is true no progress bar or non-error messages are shown.
    pub fn new(quiet: bool) -> Self {
        let bar = if quiet {
            None
        } else {
            let pb = ProgressBar::new(100);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:30.cyan/blue}] {msg}")
                    .unwrap()
                    .progress_chars("=>-"),
            );
            Some(pb)
        };
        Self { bar, quiet }
    }

    /// Announce that a new workflow phase has started.
    pub fn report_phase(&self, phase: &WorkflowPhase) {
        if self.quiet {
            return;
        }
        let label = format!("⚙  {}", phase.label());
        println!("{}", label.bold().cyan());
        if let Some(ref bar) = self.bar {
            bar.set_message(phase.label().to_string());
            let pct = match phase {
                WorkflowPhase::Scanning => 10,
                WorkflowPhase::Discovering => 30,
                WorkflowPhase::Analyzing => 50,
                WorkflowPhase::Generating => 70,
                WorkflowPhase::Validating => 90,
                WorkflowPhase::Complete => 100,
            };
            bar.set_position(pct);
        }
    }

    /// Update progress with a `current / total` pair and an optional message.
    pub fn report_progress(&self, current: u64, total: u64) {
        if self.quiet {
            return;
        }
        if let Some(ref bar) = self.bar {
            if total > 0 {
                let pct = ((current as f64 / total as f64) * 100.0) as u64;
                bar.set_position(pct.min(100));
            }
            bar.set_message(format!("{}/{}", current, total));
        }
    }

    /// Print a success message (green checkmark).
    pub fn report_success(&self, msg: &str) {
        if self.quiet {
            return;
        }
        println!("{} {}", "✔".green().bold(), msg);
    }

    /// Print a warning message (yellow).
    pub fn report_warning(&self, msg: &str) {
        // Warnings are shown even in quiet mode.
        eprintln!("{} {}", "⚠".yellow().bold(), msg.yellow());
    }

    /// Print an error message (red, to stderr).
    pub fn report_error(&self, msg: &str) {
        // Errors are always shown.
        eprintln!("{} {}", "✘".red().bold(), msg.red());
    }

    /// Print an informational message (dimmed).
    pub fn report_info(&self, msg: &str) {
        if self.quiet {
            return;
        }
        println!("{} {}", "ℹ".dimmed(), msg.dimmed());
    }

    /// Whether the reporter is in quiet mode.
    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    /// Finish the progress bar (hide it and print a completion line).
    pub fn finish(&self, success: bool) {
        if let Some(ref bar) = self.bar {
            if success {
                bar.set_style(
                    ProgressStyle::default_bar()
                        .template("{msg}")
                        .unwrap(),
                );
                bar.finish_with_message(format!("{} Done", "✔".green()));
            } else {
                bar.finish_with_message(format!("{} Aborted", "✘".red()));
            }
        }
    }
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reporter_quiet_mode_no_bar() {
        let r = Reporter::new(true);
        assert!(r.bar.is_none());
        assert!(r.quiet);
    }

    #[test]
    fn reporter_normal_mode_has_bar() {
        let r = Reporter::new(false);
        assert!(r.bar.is_some());
        assert!(!r.quiet);
    }

    #[test]
    fn report_phase_quiet_no_output() {
        let r = Reporter::new(true);
        r.report_phase(&WorkflowPhase::Scanning);
        r.report_phase(&WorkflowPhase::Complete);
        // No panic — just ensure it doesn't crash.
    }

    #[test]
    fn report_messages_quiet_suppresses_info() {
        let r = Reporter::new(true);
        // These should not panic even in quiet mode.
        r.report_success("ok");
        r.report_warning("warn");
        r.report_error("err");
        r.report_info("info");
    }

    #[test]
    fn progress_zero_total() {
        let r = Reporter::new(false);
        // Should not divide by zero.
        r.report_progress(0, 0);
    }
}
