use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Thread-safe progress tracker for batch builds
#[derive(Clone)]
pub struct Progress {
    total: usize,
    completed: Arc<AtomicUsize>,
    succeeded: Arc<AtomicUsize>,
    failed: Arc<AtomicUsize>,
    start_time: Instant,
}

impl Progress {
    pub fn new(total: usize) -> Self {
        Self {
            total,
            completed: Arc::new(AtomicUsize::new(0)),
            succeeded: Arc::new(AtomicUsize::new(0)),
            failed: Arc::new(AtomicUsize::new(0)),
            start_time: Instant::now(),
        }
    }

    pub fn increment_completed(&self, success: bool) {
        self.completed.fetch_add(1, Ordering::SeqCst);
        if success {
            self.succeeded.fetch_add(1, Ordering::SeqCst);
        } else {
            self.failed.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn completed(&self) -> usize {
        self.completed.load(Ordering::SeqCst)
    }

    pub fn succeeded(&self) -> usize {
        self.succeeded.load(Ordering::SeqCst)
    }

    pub fn failed(&self) -> usize {
        self.failed.load(Ordering::SeqCst)
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    pub fn eta_secs(&self) -> Option<f64> {
        let completed = self.completed();
        if completed == 0 {
            return None;
        }
        let elapsed = self.elapsed_secs();
        let per_item = elapsed / completed as f64;
        let remaining = self.total - completed;
        Some(per_item * remaining as f64)
    }

    pub fn print_header(&self, num_workers: usize) {
        println!("  {} maps | {} workers", self.total, num_workers);
        println!();
    }

    pub fn print_progress(&self, map_name: &str, _index: usize, status: &str, elapsed: f64) {
        let completed = self.completed();
        let pct = if self.total > 0 {
            completed as f64 / self.total as f64 * 100.0
        } else {
            0.0
        };
        
        let eta_str = if let Some(eta) = self.eta_secs() {
            format!(" ETA: {:.0}s", eta)
        } else {
            String::new()
        };

        eprintln!(
            "  [{:3.0}%] [{}/{}] {} - {:.1}s - {}{}",
            pct, completed, self.total, map_name, elapsed, status, eta_str
        );
    }

    pub fn print_summary(&self, failed_maps: &[(String, String)]) {
        let elapsed = self.elapsed_secs();
        eprintln!();
        eprintln!("  {}", "=".repeat(70));
        eprintln!("  Build complete in {:.1}s", elapsed);
        eprintln!("  ✓ {} succeeded | ✗ {} failed | {} total", 
            self.succeeded(), self.failed(), self.total);

        if !failed_maps.is_empty() {
            eprintln!();
            eprintln!("  Failed maps:");
            for (name, error) in failed_maps {
                eprintln!("    ✗ {}", name);
                // Show first line of error, indented
                for line in error.lines().take(3) {
                    eprintln!("      {}", line);
                }
                if error.lines().count() > 3 {
                    eprintln!("      ... ({} more lines)", error.lines().count() - 3);
                }
            }
        }
        eprintln!("  {}", "=".repeat(70));
    }
}
