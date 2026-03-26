//! 指标收集与导出模块
//!
//! 提供四种指标类型：
//! - Counter（计数器）：只增不减，适合统计请求数、错误数等
//! - Gauge（仪表盘）：可增可减，适合统计当前连接数、队列长度等
//! - Histogram（直方图）：按桶分布统计，适合统计延迟分布
//! - Summary（摘要）：记录原始值用于百分位计算
//!
//! 支持注册动态 Gauge 函数和导出 Prometheus 格式文本。

use indexmap::IndexMap;
use parking_lot::Mutex;

/// 直方图值：按预定义桶边界统计分布
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HistogramValue {
    pub buckets: Vec<u64>,
    pub counts: Vec<u64>,
    pub sum: u64,
    pub count: u64,
}

/// 摘要值：保留最近 max_size 个原始值，用于计算百分位
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

/// 指标快照：某一时刻所有指标的不可变副本
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MetricsSnapshot {
    pub counters: IndexMap<String, u64>,
    pub gauges: IndexMap<String, i64>,
    pub histograms: IndexMap<String, HistogramValue>,
    pub summaries: IndexMap<String, SummaryValue>,
}

/// 指标收集器
///
/// 线程安全的指标注册与收集中心，支持动态 Gauge 函数注册。
/// 所有指标名称会自动加上 prefix 前缀（默认 "anima"）。
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

    /// 预注册 Agent 运行时常用的指标（计数器、仪表盘、直方图）
    ///
    /// 包括消息处理、任务执行、缓存命中、Worker 状态等核心指标，
    /// 以及消息延迟和任务耗时的直方图桶配置。
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

    /// 记录直方图观测值，自动落入对应的桶
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

    /// 注册摘要指标，可指定保留的最大样本数
    pub fn register_summary(&self, name: &str, max_size: Option<usize>) {
        self.summaries.lock().entry(name.into()).or_insert_with(|| SummaryValue {
            max_size: max_size.unwrap_or(1000),
            ..Default::default()
        });
    }

    /// 记录摘要观测值，超过 max_size 时以环形缓冲方式覆盖最旧的值
    pub fn summary_record(&self, name: &str, value: f64) {
        let mut summaries = self.summaries.lock();
        let summary = summaries.entry(name.into()).or_default();
        summary.count += 1;
        summary.sum += value;
        if summary.values.len() < summary.max_size {
            summary.values.push(value);
        } else if !summary.values.is_empty() {
            // 环形缓冲：用 count 取模定位覆盖位置
            let idx = (summary.count as usize - 1) % summary.max_size;
            summary.values[idx] = value;
        }
    }

    pub fn summary_get(&self, name: &str) -> Option<SummaryValue> {
        self.summaries.lock().get(name).cloned()
    }

    /// 注册动态 Gauge 函数，snapshot 时会自动调用获取最新值
    pub fn register_gauge_fn(&self, name: &str, value_fn: Box<dyn Fn() -> i64 + Send + Sync>) {
        self.gauge_fns.lock().insert(name.into(), value_fn);
    }

    /// 生成当前所有指标的不可变快照（会触发动态 Gauge 函数求值）
    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut gauges = self.gauges.lock().clone();
        // 执行所有注册的动态 Gauge 函数，将结果合并到快照中
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

    /// 导出 Prometheus 文本格式的指标数据，可直接用于 /metrics 端点
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
