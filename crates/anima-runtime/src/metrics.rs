use indexmap::IndexMap;
use parking_lot::Mutex;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HistogramValue {
    pub buckets: Vec<u64>,
    pub counts: Vec<u64>,
    pub sum: u64,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummaryValue {
    pub values: Vec<f64>,
    pub max_size: usize,
    pub count: u64,
    pub sum: f64,
}

impl Default for SummaryValue {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            max_size: 1000,
            count: 0,
            sum: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetricsSnapshot {
    pub counters: IndexMap<String, u64>,
    pub gauges: IndexMap<String, i64>,
    pub histograms: IndexMap<String, HistogramValue>,
    pub summaries: IndexMap<String, SummaryValue>,
}

pub struct MetricsCollector {
    prefix: String,
    counters: Mutex<IndexMap<String, u64>>,
    gauges: Mutex<IndexMap<String, i64>>,
    histograms: Mutex<IndexMap<String, HistogramValue>>,
    summaries: Mutex<IndexMap<String, SummaryValue>>,
    gauge_fns: Mutex<IndexMap<String, Box<dyn Fn() -> i64 + Send + Sync>>>,
}

impl std::fmt::Debug for MetricsCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsCollector")
            .field("prefix", &self.prefix)
            .finish()
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new(None)
    }
}

impl MetricsCollector {
    pub fn new(prefix: Option<&str>) -> Self {
        Self {
            prefix: prefix.unwrap_or("anima").to_string(),
            counters: Mutex::new(IndexMap::new()),
            gauges: Mutex::new(IndexMap::new()),
            histograms: Mutex::new(IndexMap::new()),
            summaries: Mutex::new(IndexMap::new()),
            gauge_fns: Mutex::new(IndexMap::new()),
        }
    }

    pub fn register_agent_metrics(&self) {
        for key in [
            "messages_received",
            "messages_processed",
            "messages_failed",
            "tasks_submitted",
            "tasks_completed",
            "tasks_failed",
            "cache_hits",
            "cache_misses",
            "cache_evictions",
            "workers_tasks_total",
        ] {
            self.counters.lock().entry(key.into()).or_insert(0);
        }
        for key in ["sessions_active", "workers_active", "workers_idle"] {
            self.gauges.lock().entry(key.into()).or_insert(0);
        }
        self.histograms
            .lock()
            .entry("message_latency".into())
            .or_insert_with(|| HistogramValue {
                buckets: vec![10, 50, 100, 250, 500, 1000, 2500, 5000],
                counts: vec![0; 9],
                sum: 0,
                count: 0,
            });
        self.histograms
            .lock()
            .entry("task_duration".into())
            .or_insert_with(|| HistogramValue {
                buckets: vec![100, 500, 1000, 5000, 10000, 30000, 60000],
                counts: vec![0; 8],
                sum: 0,
                count: 0,
            });
    }

    pub fn counter_inc(&self, name: &str) {
        *self.counters.lock().entry(name.into()).or_insert(0) += 1;
    }

    pub fn counter_add(&self, name: &str, value: u64) {
        *self.counters.lock().entry(name.into()).or_insert(0) += value;
    }

    pub fn histogram_record(&self, name: &str, value: u64) {
        let mut histograms = self.histograms.lock();
        let histogram = histograms
            .entry(name.into())
            .or_insert_with(|| HistogramValue {
                buckets: Vec::new(),
                counts: vec![0],
                sum: 0,
                count: 0,
            });
        if histogram.counts.is_empty() {
            histogram.counts = vec![0; histogram.buckets.len() + 1];
        }
        histogram.sum += value;
        histogram.count += 1;
        let mut bucket_idx = histogram.buckets.len();
        for (idx, bucket) in histogram.buckets.iter().enumerate() {
            if value <= *bucket {
                bucket_idx = idx;
                break;
            }
        }
        if histogram.counts.len() <= bucket_idx {
            histogram.counts.resize(bucket_idx + 1, 0);
        }
        histogram.counts[bucket_idx] += 1;
    }

    pub fn update_session_gauge(&self, count: usize) {
        self.gauges
            .lock()
            .insert("sessions_active".into(), count as i64);
    }

    pub fn update_worker_gauges(&self, active: usize, idle: usize) {
        let mut gauges = self.gauges.lock();
        gauges.insert("workers_active".into(), active as i64);
        gauges.insert("workers_idle".into(), idle as i64);
    }

    pub fn register_summary(&self, name: &str, max_size: Option<usize>) {
        self.summaries.lock().entry(name.into()).or_insert_with(|| SummaryValue {
            max_size: max_size.unwrap_or(1000),
            ..Default::default()
        });
    }

    pub fn summary_record(&self, name: &str, value: f64) {
        let mut summaries = self.summaries.lock();
        let summary = summaries.entry(name.into()).or_default();
        summary.count += 1;
        summary.sum += value;
        if summary.values.len() < summary.max_size {
            summary.values.push(value);
        } else if !summary.values.is_empty() {
            // Rotate: replace oldest
            let idx = (summary.count as usize - 1) % summary.max_size;
            summary.values[idx] = value;
        }
    }

    pub fn summary_get(&self, name: &str) -> Option<SummaryValue> {
        self.summaries.lock().get(name).cloned()
    }

    pub fn register_gauge_fn(&self, name: &str, value_fn: Box<dyn Fn() -> i64 + Send + Sync>) {
        self.gauge_fns.lock().insert(name.into(), value_fn);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut gauges = self.gauges.lock().clone();
        // Evaluate gauge functions
        for (name, func) in self.gauge_fns.lock().iter() {
            gauges.insert(name.clone(), func());
        }
        MetricsSnapshot {
            counters: self.counters.lock().clone(),
            gauges,
            histograms: self.histograms.lock().clone(),
            summaries: self.summaries.lock().clone(),
        }
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn render_prometheus(&self) -> String {
        let snapshot = self.snapshot();
        let mut lines = Vec::new();

        for (name, value) in snapshot.counters {
            let metric = format!("{}_{}", self.prefix, name);
            lines.push(format!("# TYPE {} counter", metric));
            lines.push(format!("{} {}", metric, value));
        }

        for (name, value) in snapshot.gauges {
            let metric = format!("{}_{}", self.prefix, name);
            lines.push(format!("# TYPE {} gauge", metric));
            lines.push(format!("{} {}", metric, value));
        }

        for (name, histogram) in snapshot.histograms {
            let metric = format!("{}_{}", self.prefix, name);
            lines.push(format!("# TYPE {} histogram", metric));
            let mut cumulative = 0u64;
            for (idx, bucket) in histogram.buckets.iter().enumerate() {
                cumulative += histogram.counts.get(idx).copied().unwrap_or(0);
                lines.push(format!("{}_bucket{{le=\"{}\"}} {}", metric, bucket, cumulative));
            }
            cumulative += histogram.counts.last().copied().unwrap_or(0);
            lines.push(format!("{}_bucket{{le=\"+Inf\"}} {}", metric, cumulative));
            lines.push(format!("{}_sum {}", metric, histogram.sum));
            lines.push(format!("{}_count {}", metric, histogram.count));
        }

        for (name, summary) in snapshot.summaries {
            let metric = format!("{}_{}", self.prefix, name);
            lines.push(format!("# TYPE {} summary", metric));
            lines.push(format!("{}_sum {}", metric, summary.sum));
            lines.push(format!("{}_count {}", metric, summary.count));
        }

        lines.join("\n")
    }
}
