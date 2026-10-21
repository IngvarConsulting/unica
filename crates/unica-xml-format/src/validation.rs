//! Exact shared XML diagnostic counter and text collector.
pub struct XmlValidationReporter {
    pub errors: usize,
    pub warnings: usize,
    pub ok_count: usize,
    pub stopped: bool,
    pub max_errors: usize,
    pub detailed: bool,
    pub lines: Vec<String>,
}

impl XmlValidationReporter {
    pub fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: Vec::new(),
        }
    }

    pub fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }
}
