//! Adaptive parameter optimization
//!
//! This module provides general-purpose parameter optimization for benchmark workloads.
//! It tunes configurable parameters (clients, threads, pipeline, ef_search) to optimize
//! for a target objective (maximize QPS, minimize latency) while satisfying constraints.
//!
//! # Design
//!
//! The optimizer is workload-agnostic. Vector search is just a special case where
//! ef_search is a tunable parameter with recall as a constraint. For simple GET/SET
//! workloads, you might tune clients and threads to maximize QPS with latency constraints.
//!
//! The optimization uses a phased approach:
//! 1. **Feasibility**: Verify the system works and constraints can be met
//! 2. **Exploration**: Grid search over parameter space to find promising regions
//! 3. **Exploitation**: Fine-tune parameters around the best known configuration
//!
//! # Example Usage
//!
//! ```ignore
//! // Vector search: maximize QPS with recall > 0.95
//! let optimizer = Optimizer::builder()
//!     .objective(Objective::parse("maximize:qps")?)
//!     .constraint(Constraint::parse("recall:gt:0.95")?)
//!     .constraint(Constraint::parse("p99_ms:lt:10")?)
//!     .parameter(TunableParameter::parse("ef_search:10:500:10")?)
//!     .build();
//!
//! // GET benchmark: maximize QPS with p99 < 0.1ms (100us)
//! let optimizer = Optimizer::builder()
//!     .objective(Objective::parse("maximize:qps")?)
//!     .constraint(Constraint::parse("p99_ms:lt:0.1")?)
//!     .parameter(TunableParameter::parse("clients:10:200:10")?)
//!     .parameter(TunableParameter::parse("threads:1:16:1")?)
//!     .build();
//! ```

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::benchmark::BenchmarkResult;

// ============================================================================
// Metric - What we measure
// ============================================================================

/// Metrics that can be measured from a benchmark run
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Throughput (requests per second)
    Qps,
    /// Average recall (0.0 - 1.0) - for vector search
    Recall,
    /// p50 latency in milliseconds
    P50Ms,
    /// p95 latency in milliseconds
    P95Ms,
    /// p99 latency in milliseconds
    P99Ms,
    /// p99.9 latency in milliseconds
    P999Ms,
    /// Mean latency in milliseconds
    MeanLatencyMs,
    /// Maximum latency in milliseconds
    MaxLatencyMs,
    /// Error rate (0.0 - 1.0)
    ErrorRate,
}

impl Metric {
    /// Extract metric value from benchmark result
    pub fn extract(&self, result: &BenchmarkResult) -> f64 {
        match self {
            Metric::Qps => result.throughput,
            Metric::Recall => result.recall_stats.average(),
            Metric::P50Ms => result.percentile_ms(50.0),
            Metric::P95Ms => result.percentile_ms(95.0),
            Metric::P99Ms => result.percentile_ms(99.0),
            Metric::P999Ms => result.percentile_ms(99.9),
            Metric::MeanLatencyMs => result.histogram.mean() / 1000.0,
            Metric::MaxLatencyMs => result.histogram.max() as f64 / 1000.0,
            Metric::ErrorRate => {
                if result.total_requests > 0 {
                    result.error_count as f64 / result.total_requests as f64
                } else {
                    0.0
                }
            }
        }
    }

    /// Get display name
    pub fn name(&self) -> &'static str {
        match self {
            Metric::Qps => "qps",
            Metric::Recall => "recall",
            Metric::P50Ms => "p50_ms",
            Metric::P95Ms => "p95_ms",
            Metric::P99Ms => "p99_ms",
            Metric::P999Ms => "p999_ms",
            Metric::MeanLatencyMs => "mean_latency_ms",
            Metric::MaxLatencyMs => "max_latency_ms",
            Metric::ErrorRate => "error_rate",
        }
    }
}

impl FromStr for Metric {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "qps" | "throughput" => Ok(Metric::Qps),
            "recall" | "recall_avg" => Ok(Metric::Recall),
            "p50" | "p50_ms" => Ok(Metric::P50Ms),
            "p95" | "p95_ms" => Ok(Metric::P95Ms),
            "p99" | "p99_ms" => Ok(Metric::P99Ms),
            "p999" | "p999_ms" => Ok(Metric::P999Ms),
            "mean" | "latency" | "mean_latency" | "mean_latency_ms" | "avg_latency" => Ok(Metric::MeanLatencyMs),
            "max" | "max_latency" | "max_latency_ms" => Ok(Metric::MaxLatencyMs),
            "error_rate" | "errors" => Ok(Metric::ErrorRate),
            _ => Err(format!("Unknown metric: '{}'. Valid: qps, recall, p50_ms, p95_ms, p99_ms, p999_ms, mean_latency_ms, max_latency_ms, error_rate", s)),
        }
    }
}

impl fmt::Display for Metric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ============================================================================
// Comparison operators
// ============================================================================

/// Comparison operator for constraints
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Gt,  // >
    Gte, // >=
    Lt,  // <
    Lte, // <=
    Eq,  // ==
}

impl CompareOp {
    const EPSILON: f64 = 1e-9;

    /// Evaluate the comparison
    pub fn evaluate(&self, actual: f64, target: f64) -> bool {
        match self {
            CompareOp::Gt => actual > target,
            CompareOp::Gte => actual >= target - Self::EPSILON,
            CompareOp::Lt => actual < target,
            CompareOp::Lte => actual <= target + Self::EPSILON,
            CompareOp::Eq => (actual - target).abs() < Self::EPSILON,
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            CompareOp::Gt => ">",
            CompareOp::Gte => ">=",
            CompareOp::Lt => "<",
            CompareOp::Lte => "<=",
            CompareOp::Eq => "==",
        }
    }
}

impl FromStr for CompareOp {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "gt" | ">" => Ok(CompareOp::Gt),
            "gte" | ">=" | "ge" => Ok(CompareOp::Gte),
            "lt" | "<" => Ok(CompareOp::Lt),
            "lte" | "<=" | "le" => Ok(CompareOp::Lte),
            "eq" | "==" | "=" => Ok(CompareOp::Eq),
            _ => Err(format!("Unknown operator: '{}'. Valid: gt, gte, lt, lte, eq", s)),
        }
    }
}

// ============================================================================
// Constraint - Conditions that must be satisfied
// ============================================================================

/// A constraint that must be satisfied
#[derive(Debug, Clone)]
pub struct Constraint {
    pub metric: Metric,
    pub op: CompareOp,
    pub value: f64,
}

impl Constraint {
    pub fn new(metric: Metric, op: CompareOp, value: f64) -> Self {
        Self { metric, op, value }
    }

    /// Convenience constructors
    pub fn gt(metric: Metric, value: f64) -> Self {
        Self::new(metric, CompareOp::Gt, value)
    }

    pub fn gte(metric: Metric, value: f64) -> Self {
        Self::new(metric, CompareOp::Gte, value)
    }

    pub fn lt(metric: Metric, value: f64) -> Self {
        Self::new(metric, CompareOp::Lt, value)
    }

    pub fn lte(metric: Metric, value: f64) -> Self {
        Self::new(metric, CompareOp::Lte, value)
    }

    /// Check if constraint is satisfied
    pub fn is_satisfied(&self, result: &BenchmarkResult) -> bool {
        let actual = self.metric.extract(result);
        self.op.evaluate(actual, self.value)
    }

    /// Get actual value for this constraint from result
    pub fn actual_value(&self, result: &BenchmarkResult) -> f64 {
        self.metric.extract(result)
    }

    /// Parse constraint from string: "metric:op:value"
    /// Examples: "recall:gt:0.95", "p99_ms:lt:0.1", "qps:gte:100000"
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return Err(format!(
                "Invalid constraint format: '{}'. Expected 'metric:op:value' (e.g., 'recall:gt:0.95', 'p99_ms:lt:0.1')",
                s
            ));
        }

        let metric = parts[0].parse::<Metric>()?;
        let op = parts[1].parse::<CompareOp>()?;
        let value = parts[2]
            .parse::<f64>()
            .map_err(|e| format!("Invalid value '{}': {}", parts[2], e))?;

        Ok(Self::new(metric, op, value))
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.metric, self.op.symbol(), self.value)
    }
}

// ============================================================================
// Objective - What we're optimizing for (supports multi-objective with tolerance)
// ============================================================================

/// Direction for optimization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizeDirection {
    Maximize,
    Minimize,
}

impl OptimizeDirection {
    /// Returns true if a is better than b for this direction
    pub fn is_better(&self, a: f64, b: f64) -> bool {
        match self {
            OptimizeDirection::Maximize => a > b,
            OptimizeDirection::Minimize => a < b,
        }
    }

    /// Returns true if a is significantly better than b (beyond tolerance)
    pub fn is_significantly_better(&self, a: f64, b: f64, tolerance: f64) -> bool {
        match self {
            OptimizeDirection::Maximize => a > b * (1.0 + tolerance),
            OptimizeDirection::Minimize => a < b * (1.0 - tolerance),
        }
    }

    /// Returns true if a and b are within tolerance of each other
    pub fn is_equivalent(&self, a: f64, b: f64, tolerance: f64) -> bool {
        if b == 0.0 {
            return a == 0.0;
        }
        let ratio = a / b;
        ratio >= (1.0 - tolerance) && ratio <= (1.0 + tolerance)
    }
}

impl fmt::Display for OptimizeDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptimizeDirection::Maximize => write!(f, "maximize"),
            OptimizeDirection::Minimize => write!(f, "minimize"),
        }
    }
}

/// A single optimization goal
///
/// Examples:
/// - `maximize:qps` - maximize QPS
/// - `minimize:p99_ms` - minimize p99 latency
/// - `maximize:qps:lt:1000000` - maximize QPS where QPS < 1M (bounded)
/// - `minimize:p99_ms:gt:0.1` - minimize p99 where p99 > 0.1ms (bounded)
#[derive(Debug, Clone)]
pub struct ObjectiveGoal {
    pub direction: OptimizeDirection,
    pub metric: Metric,
    /// Optional bound that must be satisfied for this goal to apply
    /// Example: For "maximize:qps:lt:1000000", bound = Some((Lt, 1000000.0))
    pub bound: Option<(CompareOp, f64)>,
}

impl ObjectiveGoal {
    pub fn new(direction: OptimizeDirection, metric: Metric) -> Self {
        Self {
            direction,
            metric,
            bound: None,
        }
    }

    pub fn with_bound(mut self, op: CompareOp, value: f64) -> Self {
        self.bound = Some((op, value));
        self
    }

    /// Check if this goal's bound is satisfied (if any)
    pub fn bound_satisfied(&self, value: f64) -> bool {
        match &self.bound {
            Some((op, target)) => op.evaluate(value, *target),
            None => true,
        }
    }

    /// Extract the metric value from a result
    pub fn extract(&self, result: &BenchmarkResult) -> f64 {
        self.metric.extract(result)
    }

    /// Parse a single goal from string
    /// Formats:
    /// - "maximize:qps" or "max:qps"
    /// - "minimize:p99_ms" or "min:p99_ms"
    /// - "maximize:qps:lt:1000000" (bounded)
    /// - "minimize:p99_ms:gt:0.1" (bounded)
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() < 2 {
            return Err(format!(
                "Invalid goal format: '{}'. Expected 'maximize:metric' or 'minimize:metric[:op:value]'",
                s
            ));
        }

        let direction = match parts[0].to_lowercase().as_str() {
            "maximize" | "max" => OptimizeDirection::Maximize,
            "minimize" | "min" => OptimizeDirection::Minimize,
            _ => {
                return Err(format!(
                    "Invalid direction: '{}'. Expected 'maximize' or 'minimize'",
                    parts[0]
                ))
            }
        };

        let metric = parts[1].parse::<Metric>()?;
        let mut goal = ObjectiveGoal::new(direction, metric);

        // Parse optional bound: :op:value
        if parts.len() >= 4 {
            let op = parts[2].parse::<CompareOp>()?;
            let value = parts[3]
                .parse::<f64>()
                .map_err(|e| format!("Invalid bound value '{}': {}", parts[3], e))?;
            goal = goal.with_bound(op, value);
        } else if parts.len() == 3 {
            return Err(format!(
                "Invalid goal format: '{}'. Bound requires both operator and value (e.g., ':lt:1000000')",
                s
            ));
        }

        Ok(goal)
    }
}

impl fmt::Display for ObjectiveGoal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.direction, self.metric)?;
        if let Some((op, value)) = &self.bound {
            write!(f, ":{}:{}", op.symbol(), value)?;
        }
        Ok(())
    }
}

/// Ordered list of optimization goals with tolerance for equivalence
///
/// Goals are evaluated in order:
/// 1. First, all constraints must be satisfied
/// 2. For each goal in order:
///    - If values differ significantly (beyond tolerance), better value wins
///    - If within tolerance, move to next goal as tiebreaker
///
/// Example: `maximize:qps,minimize:p99_ms` with 4% tolerance means:
/// - Primary goal: maximize QPS
/// - If two configs have QPS within 4% of each other, prefer lower p99
#[derive(Debug, Clone)]
pub struct Objectives {
    /// Ordered goals - first is primary, rest are tiebreakers
    pub goals: Vec<ObjectiveGoal>,
    /// Tolerance for equivalence (0.04 = 4%)
    pub tolerance: f64,
}

impl Objectives {
    /// Default tolerance: 4%
    pub const DEFAULT_TOLERANCE: f64 = 0.04;

    pub fn new(goals: Vec<ObjectiveGoal>) -> Self {
        Self {
            goals,
            tolerance: Self::DEFAULT_TOLERANCE,
        }
    }

    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance.clamp(0.0, 1.0);
        self
    }

    /// Get the primary (first) goal
    pub fn primary(&self) -> Option<&ObjectiveGoal> {
        self.goals.first()
    }

    /// Get the primary metric
    pub fn primary_metric(&self) -> Metric {
        self.goals.first().map(|g| g.metric).unwrap_or(Metric::Qps)
    }

    /// Extract primary objective value from result
    pub fn primary_value(&self, result: &BenchmarkResult) -> f64 {
        self.goals
            .first()
            .map(|g| g.extract(result))
            .unwrap_or(0.0)
    }

    /// Check if all goal bounds are satisfied
    pub fn bounds_satisfied(&self, metrics: &HashMap<Metric, f64>) -> bool {
        self.goals.iter().all(|g| {
            let value = metrics.get(&g.metric).copied().unwrap_or(0.0);
            g.bound_satisfied(value)
        })
    }

    /// Compare two measurements and return true if `a` is better than `b`
    ///
    /// Comparison rules:
    /// 1. Both must satisfy constraints (handled externally)
    /// 2. Both must satisfy goal bounds
    /// 3. For each goal in order:
    ///    - If a is significantly better (beyond tolerance), a wins
    ///    - If b is significantly better (beyond tolerance), b wins
    ///    - If within tolerance, continue to next goal
    /// 4. If all goals are equivalent, prefer the existing best (b)
    pub fn is_better(&self, a_metrics: &HashMap<Metric, f64>, b_metrics: &HashMap<Metric, f64>) -> bool {
        // Check bounds for both
        let a_bounds_ok = self.bounds_satisfied(a_metrics);
        let b_bounds_ok = self.bounds_satisfied(b_metrics);

        // If only one satisfies bounds, that one wins
        if a_bounds_ok && !b_bounds_ok {
            return true;
        }
        if !a_bounds_ok && b_bounds_ok {
            return false;
        }
        // If neither satisfies bounds, prefer the one closer to satisfying
        if !a_bounds_ok && !b_bounds_ok {
            // Fall back to primary goal comparison without bounds
            if let Some(goal) = self.goals.first() {
                let a_val = a_metrics.get(&goal.metric).copied().unwrap_or(0.0);
                let b_val = b_metrics.get(&goal.metric).copied().unwrap_or(0.0);
                return goal.direction.is_better(a_val, b_val);
            }
            return false;
        }

        // Both satisfy bounds - compare goals in order
        for goal in &self.goals {
            let a_val = a_metrics.get(&goal.metric).copied().unwrap_or(0.0);
            let b_val = b_metrics.get(&goal.metric).copied().unwrap_or(0.0);

            // Check if significantly different
            if goal.direction.is_significantly_better(a_val, b_val, self.tolerance) {
                return true; // a is clearly better for this goal
            }
            if goal.direction.is_significantly_better(b_val, a_val, self.tolerance) {
                return false; // b is clearly better for this goal
            }
            // Within tolerance, continue to next goal as tiebreaker
        }

        // All goals are within tolerance with no tiebreaker available
        // Fall back to strict comparison on primary goal - prefer the strictly better value
        if let Some(goal) = self.goals.first() {
            let a_val = a_metrics.get(&goal.metric).copied().unwrap_or(0.0);
            let b_val = b_metrics.get(&goal.metric).copied().unwrap_or(0.0);
            return goal.direction.is_better(a_val, b_val);
        }

        false
    }

    /// Parse objectives from string: "goal1,goal2,..."
    /// Examples:
    /// - "maximize:qps"
    /// - "maximize:qps,minimize:p99_ms"
    /// - "maximize:qps:lt:1000000,minimize:p99_ms"
    pub fn parse(s: &str) -> Result<Self, String> {
        let goal_strs: Vec<&str> = s.split(',').map(|s| s.trim()).collect();
        if goal_strs.is_empty() {
            return Err("No objectives specified".to_string());
        }

        let mut goals = Vec::new();
        for goal_str in goal_strs {
            goals.push(ObjectiveGoal::parse(goal_str)?);
        }

        Ok(Self::new(goals))
    }
}

impl Default for Objectives {
    fn default() -> Self {
        Self::new(vec![ObjectiveGoal::new(
            OptimizeDirection::Maximize,
            Metric::Qps,
        )])
    }
}

impl fmt::Display for Objectives {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let goals: Vec<String> = self.goals.iter().map(|g| g.to_string()).collect();
        write!(f, "{}", goals.join(", "))?;
        if (self.tolerance - Self::DEFAULT_TOLERANCE).abs() > 0.001 {
            write!(f, " (tolerance: {:.1}%)", self.tolerance * 100.0)?;
        }
        Ok(())
    }
}

// Legacy Objective type for backwards compatibility
// TODO: Remove once all code is migrated to Objectives
/// Optimization objective (legacy - use Objectives for multi-goal)
#[derive(Debug, Clone)]
pub enum Objective {
    /// Maximize a metric (e.g., QPS)
    Maximize(Metric),
    /// Minimize a metric (e.g., p99 latency)
    Minimize(Metric),
}

impl Objective {
    pub fn metric(&self) -> Metric {
        match self {
            Objective::Maximize(m) | Objective::Minimize(m) => *m,
        }
    }

    /// Returns true if a is better than b for this objective
    pub fn is_better(&self, a: f64, b: f64) -> bool {
        match self {
            Objective::Maximize(_) => a > b,
            Objective::Minimize(_) => a < b,
        }
    }

    pub fn extract(&self, result: &BenchmarkResult) -> f64 {
        self.metric().extract(result)
    }

    /// Convert to Objectives (multi-goal)
    pub fn to_objectives(&self) -> Objectives {
        let goal = match self {
            Objective::Maximize(m) => ObjectiveGoal::new(OptimizeDirection::Maximize, *m),
            Objective::Minimize(m) => ObjectiveGoal::new(OptimizeDirection::Minimize, *m),
        };
        Objectives::new(vec![goal])
    }

    /// Parse objective from string: "maximize:metric" or "minimize:metric"
    /// Also supports multi-goal: "maximize:qps,minimize:p99_ms"
    pub fn parse(s: &str) -> Result<Self, String> {
        // Check if it's multi-goal (contains comma)
        if s.contains(',') {
            // Parse as Objectives and convert first goal to Objective
            let objectives = Objectives::parse(s)?;
            if let Some(goal) = objectives.goals.first() {
                return match goal.direction {
                    OptimizeDirection::Maximize => Ok(Objective::Maximize(goal.metric)),
                    OptimizeDirection::Minimize => Ok(Objective::Minimize(goal.metric)),
                };
            }
        }

        // Single goal - try parsing as ObjectiveGoal first (supports bounds)
        if let Ok(goal) = ObjectiveGoal::parse(s) {
            return match goal.direction {
                OptimizeDirection::Maximize => Ok(Objective::Maximize(goal.metric)),
                OptimizeDirection::Minimize => Ok(Objective::Minimize(goal.metric)),
            };
        }

        // Fallback to original parsing
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() < 2 {
            return Err(format!(
                "Invalid objective format: '{}'. Expected 'maximize:metric' or 'minimize:metric'",
                s
            ));
        }

        let metric = parts[1].parse::<Metric>()?;
        match parts[0].to_lowercase().as_str() {
            "maximize" | "max" => Ok(Objective::Maximize(metric)),
            "minimize" | "min" => Ok(Objective::Minimize(metric)),
            _ => Err(format!(
                "Invalid objective type: '{}'. Expected 'maximize' or 'minimize'",
                parts[0]
            )),
        }
    }
}

impl Default for Objective {
    fn default() -> Self {
        Objective::Maximize(Metric::Qps)
    }
}

impl fmt::Display for Objective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Objective::Maximize(m) => write!(f, "maximize:{}", m),
            Objective::Minimize(m) => write!(f, "minimize:{}", m),
        }
    }
}

// ============================================================================
// TunableParameter - Parameters we can adjust
// ============================================================================

/// Types of tunable parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterType {
    Clients,
    Threads,
    Pipeline,
    EfSearch,
}

impl ParameterType {
    pub fn name(&self) -> &'static str {
        match self {
            ParameterType::Clients => "clients",
            ParameterType::Threads => "threads",
            ParameterType::Pipeline => "pipeline",
            ParameterType::EfSearch => "ef_search",
        }
    }
}

impl FromStr for ParameterType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "clients" | "c" => Ok(ParameterType::Clients),
            "threads" | "t" => Ok(ParameterType::Threads),
            "pipeline" | "p" => Ok(ParameterType::Pipeline),
            "ef_search" | "ef" | "efsearch" => Ok(ParameterType::EfSearch),
            _ => Err(format!(
                "Unknown parameter: '{}'. Valid: clients, threads, pipeline, ef_search",
                s
            )),
        }
    }
}

impl fmt::Display for ParameterType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A tunable parameter with its range and step
#[derive(Debug, Clone)]
pub struct TunableParameter {
    pub param_type: ParameterType,
    pub min: u32,
    pub max: u32,
    pub step: u32,
}

impl TunableParameter {
    pub fn new(param_type: ParameterType, min: u32, max: u32, step: u32) -> Self {
        Self { param_type, min, max, step }
    }

    /// Convenience constructors
    pub fn clients(min: u32, max: u32, step: u32) -> Self {
        Self::new(ParameterType::Clients, min, max, step)
    }

    pub fn threads(min: u32, max: u32, step: u32) -> Self {
        Self::new(ParameterType::Threads, min, max, step)
    }

    pub fn pipeline(min: u32, max: u32, step: u32) -> Self {
        Self::new(ParameterType::Pipeline, min, max, step)
    }

    pub fn ef_search(min: u32, max: u32, step: u32) -> Self {
        Self::new(ParameterType::EfSearch, min, max, step)
    }

    /// Parse from string: "type:min:max:step"
    /// Examples: "ef_search:10:500:10", "clients:10:200:10", "threads:1:16:1"
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 4 {
            return Err(format!(
                "Invalid parameter format: '{}'. Expected 'type:min:max:step' (e.g., 'clients:10:200:10')",
                s
            ));
        }

        let param_type = parts[0].parse::<ParameterType>()?;
        let min = parts[1].parse::<u32>().map_err(|e| format!("Invalid min: {}", e))?;
        let max = parts[2].parse::<u32>().map_err(|e| format!("Invalid max: {}", e))?;
        let step = parts[3].parse::<u32>().map_err(|e| format!("Invalid step: {}", e))?;

        if min > max {
            return Err(format!("min ({}) must be <= max ({})", min, max));
        }
        if step == 0 {
            return Err("step must be > 0".to_string());
        }

        Ok(Self::new(param_type, min, max, step))
    }

    /// Get number of possible values
    pub fn num_values(&self) -> u32 {
        (self.max - self.min) / self.step + 1
    }

    /// Get value at index
    pub fn value_at(&self, index: u32) -> u32 {
        (self.min + index * self.step).min(self.max)
    }

    /// Get all possible values
    pub fn values(&self) -> Vec<u32> {
        (0..self.num_values()).map(|i| self.value_at(i)).collect()
    }

    /// Get middle value (good starting point)
    pub fn mid_value(&self) -> u32 {
        self.value_at(self.num_values() / 2)
    }
}

impl fmt::Display for TunableParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:[{}-{}:{}]", self.param_type, self.min, self.max, self.step)
    }
}

// ============================================================================
// TestConfig - Configuration being tested
// ============================================================================

/// Configuration to test (parameter values)
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TestConfig {
    pub clients: Option<u32>,
    pub threads: Option<u32>,
    pub pipeline: Option<u32>,
    pub ef_search: Option<u32>,
}

impl TestConfig {
    pub fn set(&mut self, param_type: ParameterType, value: u32) {
        match param_type {
            ParameterType::Clients => self.clients = Some(value),
            ParameterType::Threads => self.threads = Some(value),
            ParameterType::Pipeline => self.pipeline = Some(value),
            ParameterType::EfSearch => self.ef_search = Some(value),
        }
    }

    pub fn get(&self, param_type: ParameterType) -> Option<u32> {
        match param_type {
            ParameterType::Clients => self.clients,
            ParameterType::Threads => self.threads,
            ParameterType::Pipeline => self.pipeline,
            ParameterType::EfSearch => self.ef_search,
        }
    }
}

impl fmt::Display for TestConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(v) = self.clients {
            parts.push(format!("clients={}", v));
        }
        if let Some(v) = self.threads {
            parts.push(format!("threads={}", v));
        }
        if let Some(v) = self.pipeline {
            parts.push(format!("pipeline={}", v));
        }
        if let Some(v) = self.ef_search {
            parts.push(format!("ef_search={}", v));
        }
        if parts.is_empty() {
            write!(f, "{{default}}")
        } else {
            write!(f, "{{{}}}", parts.join(", "))
        }
    }
}

// ============================================================================
// Measurement - Result of a single test iteration
// ============================================================================

/// Result of a single test iteration
#[derive(Debug, Clone)]
pub struct Measurement {
    pub config: TestConfig,
    pub metrics: HashMap<Metric, f64>,
    pub objective_value: f64,
    pub constraints_met: bool,
    /// (constraint, satisfied, actual_value)
    pub constraint_results: Vec<(Constraint, bool, f64)>,
}

impl Measurement {
    pub fn from_result(
        config: TestConfig,
        result: &BenchmarkResult,
        objective: &Objective,
        constraints: &[Constraint],
    ) -> Self {
        // Extract all metrics
        let mut metrics = HashMap::new();
        for metric in [
            Metric::Qps, Metric::Recall, Metric::P50Ms, Metric::P95Ms,
            Metric::P99Ms, Metric::P999Ms, Metric::MeanLatencyMs,
            Metric::MaxLatencyMs, Metric::ErrorRate,
        ] {
            metrics.insert(metric, metric.extract(result));
        }

        let objective_value = objective.extract(result);

        let constraint_results: Vec<_> = constraints
            .iter()
            .map(|c| {
                let actual = c.actual_value(result);
                let satisfied = c.is_satisfied(result);
                (c.clone(), satisfied, actual)
            })
            .collect();

        let constraints_met = constraint_results.iter().all(|(_, sat, _)| *sat);

        Self {
            config,
            metrics,
            objective_value,
            constraints_met,
            constraint_results,
        }
    }
}

// ============================================================================
// OptimizerPhase
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizerPhase {
    Feasibility,
    Exploration,
    Exploitation,
    Complete,
}

// ============================================================================
// OptimizationResult - Best result found
// ============================================================================

#[derive(Debug, Clone)]
pub struct OptimizationResult {
    pub config: TestConfig,
    pub objective_value: f64,
    pub metrics: HashMap<Metric, f64>,
}

// ============================================================================
// OptimizerBuilder
// ============================================================================

pub struct OptimizerBuilder {
    objectives: Objectives,
    constraints: Vec<Constraint>,
    parameters: Vec<TunableParameter>,
    max_iterations: u32,
    base_requests: u64,
    exploitation_multiplier: u32,
}

impl OptimizerBuilder {
    pub fn new() -> Self {
        Self {
            objectives: Objectives::default(),
            constraints: Vec::new(),
            parameters: Vec::new(),
            max_iterations: 50,
            base_requests: Optimizer::DEFAULT_BASE_REQUESTS,
            exploitation_multiplier: Optimizer::DEFAULT_EXPLOIT_MULTIPLIER,
        }
    }

    /// Set objectives (multi-goal with tolerance)
    pub fn objectives(mut self, objectives: Objectives) -> Self {
        self.objectives = objectives;
        self
    }

    /// Set single objective (legacy, converts to Objectives)
    pub fn objective(mut self, objective: Objective) -> Self {
        self.objectives = objective.to_objectives();
        self
    }

    /// Set tolerance for equivalence (0.04 = 4%)
    pub fn tolerance(mut self, tolerance: f64) -> Self {
        self.objectives.tolerance = tolerance.clamp(0.0, 1.0);
        self
    }

    pub fn constraint(mut self, constraint: Constraint) -> Self {
        self.constraints.push(constraint);
        self
    }

    pub fn parameter(mut self, param: TunableParameter) -> Self {
        self.parameters.push(param);
        self
    }

    pub fn max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = n;
        self
    }

    /// Set base request count for exploration phase
    pub fn base_requests(mut self, n: u64) -> Self {
        self.base_requests = n;
        self
    }

    /// Set multiplier for exploitation phase (longer runs for accuracy)
    pub fn exploitation_multiplier(mut self, n: u32) -> Self {
        self.exploitation_multiplier = n;
        self
    }

    pub fn build(self) -> Optimizer {
        Optimizer::with_objectives(
            self.objectives,
            self.constraints,
            self.parameters,
            self.max_iterations,
            self.base_requests,
            self.exploitation_multiplier,
        )
    }
}

impl Default for OptimizerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Optimizer - Main state machine
// ============================================================================

/// Parameter optimizer state machine
pub struct Optimizer {
    // Configuration
    objectives: Objectives,
    /// Legacy single objective (for backwards compatibility)
    objective: Objective,
    constraints: Vec<Constraint>,
    parameters: Vec<TunableParameter>,
    max_iterations: u32,

    // State
    phase: OptimizerPhase,
    iteration: u32,
    history: Vec<Measurement>,
    best_result: Option<OptimizationResult>,
    tested_configs: std::collections::HashSet<TestConfig>,

    // Search state
    grid_configs: Vec<TestConfig>,
    grid_index: usize,
    exploit_attempts: u32,

    // Adaptive duration settings
    base_requests: u64,
    exploitation_requests_multiplier: u32,

    // Convergence tracking
    hit_iteration_limit: bool,
}

impl Optimizer {
    /// Default base requests for exploration phase
    const DEFAULT_BASE_REQUESTS: u64 = 100_000;
    /// Default multiplier for exploitation phase (5x longer runs)
    const DEFAULT_EXPLOIT_MULTIPLIER: u32 = 5;

    pub fn new(
        objective: Objective,
        constraints: Vec<Constraint>,
        parameters: Vec<TunableParameter>,
        max_iterations: u32,
    ) -> Self {
        Self::with_adaptive_duration(
            objective,
            constraints,
            parameters,
            max_iterations,
            Self::DEFAULT_BASE_REQUESTS,
            Self::DEFAULT_EXPLOIT_MULTIPLIER,
        )
    }

    pub fn with_adaptive_duration(
        objective: Objective,
        constraints: Vec<Constraint>,
        parameters: Vec<TunableParameter>,
        max_iterations: u32,
        base_requests: u64,
        exploitation_requests_multiplier: u32,
    ) -> Self {
        let objectives = objective.to_objectives();
        Self::with_objectives(
            objectives,
            constraints,
            parameters,
            max_iterations,
            base_requests,
            exploitation_requests_multiplier,
        )
    }

    pub fn with_objectives(
        objectives: Objectives,
        constraints: Vec<Constraint>,
        parameters: Vec<TunableParameter>,
        max_iterations: u32,
        base_requests: u64,
        exploitation_requests_multiplier: u32,
    ) -> Self {
        let grid_configs = Self::generate_grid(&parameters);

        // Create legacy objective from first goal
        let objective = if let Some(goal) = objectives.goals.first() {
            match goal.direction {
                OptimizeDirection::Maximize => Objective::Maximize(goal.metric),
                OptimizeDirection::Minimize => Objective::Minimize(goal.metric),
            }
        } else {
            Objective::default()
        };

        Self {
            objectives,
            objective,
            constraints,
            parameters,
            max_iterations,
            phase: OptimizerPhase::Feasibility,
            iteration: 0,
            history: Vec::new(),
            best_result: None,
            tested_configs: std::collections::HashSet::new(),
            grid_configs,
            grid_index: 0,
            exploit_attempts: 0,
            base_requests,
            exploitation_requests_multiplier: exploitation_requests_multiplier.max(1),
            hit_iteration_limit: false,
        }
    }

    pub fn builder() -> OptimizerBuilder {
        OptimizerBuilder::new()
    }

    /// Generate grid search configurations
    /// Strategy: Always include boundaries (min, max) plus evenly spaced interior points
    /// This ensures we explore the full range efficiently
    fn generate_grid(parameters: &[TunableParameter]) -> Vec<TestConfig> {
        if parameters.is_empty() {
            return vec![TestConfig::default()];
        }

        let mut configs = Vec::new();
        Self::generate_grid_recursive(parameters, 0, &mut TestConfig::default(), &mut configs);
        configs
    }

    /// Get sample points for a parameter: always min, max, plus interior points
    fn sample_parameter_values(param: &TunableParameter) -> Vec<u32> {
        let all_values = param.values();
        if all_values.len() <= 5 {
            return all_values;
        }

        // Always include min and max
        let mut samples = vec![param.min, param.max];

        // Add 3 interior points (quartiles)
        let n = all_values.len();
        samples.push(all_values[n / 4]);      // 25%
        samples.push(all_values[n / 2]);      // 50%
        samples.push(all_values[3 * n / 4]);  // 75%

        // Sort and deduplicate
        samples.sort();
        samples.dedup();
        samples
    }

    fn generate_grid_recursive(
        parameters: &[TunableParameter],
        param_idx: usize,
        current: &mut TestConfig,
        configs: &mut Vec<TestConfig>,
    ) {
        if param_idx >= parameters.len() {
            configs.push(current.clone());
            return;
        }

        let param = &parameters[param_idx];
        let values = Self::sample_parameter_values(param);

        for value in values {
            current.set(param.param_type, value);
            Self::generate_grid_recursive(parameters, param_idx + 1, current, configs);
        }
    }

    // Accessors
    pub fn phase(&self) -> OptimizerPhase { self.phase }
    pub fn is_complete(&self) -> bool { self.phase == OptimizerPhase::Complete }
    pub fn best_result(&self) -> Option<&OptimizationResult> { self.best_result.as_ref() }
    pub fn history(&self) -> &[Measurement] { &self.history }
    pub fn iteration(&self) -> u32 { self.iteration }

    /// Returns true if optimization stopped due to hitting iteration limit
    /// rather than natural convergence
    pub fn hit_iteration_limit(&self) -> bool { self.hit_iteration_limit }

    /// Returns true if optimization converged naturally (completed all phases)
    pub fn converged(&self) -> bool {
        self.phase == OptimizerPhase::Complete && !self.hit_iteration_limit
    }

    /// Get recommended number of requests for current phase
    ///
    /// Returns higher values during exploitation phase for more accurate measurements
    /// when fine-tuning near the optimal configuration.
    pub fn recommended_requests(&self) -> u64 {
        match self.phase {
            OptimizerPhase::Feasibility => self.base_requests,
            OptimizerPhase::Exploration => self.base_requests,
            OptimizerPhase::Exploitation => {
                self.base_requests * self.exploitation_requests_multiplier as u64
            }
            OptimizerPhase::Complete => self.base_requests,
        }
    }

    /// Get next configuration to test, or None if complete
    pub fn next_config(&mut self) -> Option<TestConfig> {
        if self.phase == OptimizerPhase::Complete {
            return None;
        }

        // Check if we hit the iteration limit before natural completion
        if self.iteration >= self.max_iterations {
            self.hit_iteration_limit = true;
            self.phase = OptimizerPhase::Complete;
            return None;
        }

        match self.phase {
            OptimizerPhase::Feasibility => {
                // Start with MAX values to quickly find the ceiling
                // This is more efficient than starting in the middle
                let mut config = TestConfig::default();
                for param in &self.parameters {
                    config.set(param.param_type, param.max);
                }
                Some(config)
            }
            OptimizerPhase::Exploration => {
                // Grid search
                while self.grid_index < self.grid_configs.len() {
                    let config = self.grid_configs[self.grid_index].clone();
                    self.grid_index += 1;
                    if !self.tested_configs.contains(&config) {
                        return Some(config);
                    }
                }
                // Grid complete, move to exploitation
                self.phase = OptimizerPhase::Exploitation;
                self.next_config()
            }
            OptimizerPhase::Exploitation => {
                self.generate_exploit_config()
            }
            OptimizerPhase::Complete => None,
        }
    }

    /// Generate config for exploitation phase (hill climbing around best)
    /// Strategy: Try all directions with varying step sizes
    fn generate_exploit_config(&mut self) -> Option<TestConfig> {
        if self.parameters.is_empty() || self.best_result.is_none() {
            self.phase = OptimizerPhase::Complete;
            return None;
        }

        let best = self.best_result.as_ref().unwrap();

        // Try multiple step sizes: 1x, 2x, 3x the base step
        // For each parameter, try: +1x, -1x, +2x, -2x, +3x, -3x
        let num_step_sizes = 3;
        let directions_per_param = num_step_sizes * 2; // up and down for each step size
        let max_exploit_attempts = self.parameters.len() as u32 * directions_per_param as u32;

        while self.exploit_attempts < max_exploit_attempts {
            let param_idx = (self.exploit_attempts / directions_per_param as u32) as usize;
            let direction_idx = (self.exploit_attempts % directions_per_param as u32) as usize;

            if param_idx >= self.parameters.len() {
                self.phase = OptimizerPhase::Complete;
                return None;
            }

            let param = &self.parameters[param_idx];
            let step_multiplier = (direction_idx / 2) as u32 + 1; // 1, 2, or 3
            let direction = if direction_idx.is_multiple_of(2) { 1i32 } else { -1 };
            let step_size = param.step * step_multiplier;

            self.exploit_attempts += 1;

            if let Some(current_value) = best.config.get(param.param_type) {
                let new_value = if direction > 0 {
                    (current_value + step_size).min(param.max)
                } else {
                    current_value.saturating_sub(step_size).max(param.min)
                };

                if new_value != current_value {
                    let mut config = best.config.clone();
                    config.set(param.param_type, new_value);

                    if !self.tested_configs.contains(&config) {
                        return Some(config);
                    }
                }
            }
        }

        self.phase = OptimizerPhase::Complete;
        None
    }

    /// Record a benchmark result
    pub fn record_result(&mut self, config: TestConfig, result: &BenchmarkResult) {
        self.tested_configs.insert(config.clone());

        let measurement = Measurement::from_result(
            config.clone(),
            result,
            &self.objective,
            &self.constraints,
        );

        // Update best result if constraints met and objectives improved
        // Use multi-objective comparison with tolerance
        if measurement.constraints_met {
            let is_better = self.best_result.as_ref().map(|b| {
                // Use multi-objective comparison
                self.objectives.is_better(&measurement.metrics, &b.metrics)
            }).unwrap_or(true);

            if is_better {
                self.best_result = Some(OptimizationResult {
                    config: config.clone(),
                    objective_value: measurement.objective_value,
                    metrics: measurement.metrics.clone(),
                });
                // Reset exploit attempts when we find a new best
                self.exploit_attempts = 0;
            }
        }

        self.history.push(measurement);
        self.iteration += 1;

        // Advance phase
        match self.phase {
            OptimizerPhase::Feasibility => {
                // Move to exploration regardless of feasibility result
                // (exploration will find constraints-meeting configs)
                self.phase = OptimizerPhase::Exploration;
            }
            OptimizerPhase::Exploration => {
                if self.grid_index >= self.grid_configs.len() {
                    self.phase = OptimizerPhase::Exploitation;
                }
            }
            OptimizerPhase::Exploitation | OptimizerPhase::Complete => {}
        }
    }

    /// Get the objectives
    pub fn objectives(&self) -> &Objectives {
        &self.objectives
    }

    /// Format optimization summary
    pub fn summary(&self) -> String {
        let mut s = String::new();

        s.push_str("=== Optimization Summary ===\n\n");
        s.push_str(&format!("Objectives: {}\n", self.objectives));
        if self.objectives.goals.len() > 1 {
            s.push_str(&format!(
                "  (tolerance: {:.1}% - configs within this range are compared by secondary goals)\n",
                self.objectives.tolerance * 100.0
            ));
        }

        if !self.constraints.is_empty() {
            s.push_str("Constraints:\n");
            for c in &self.constraints {
                s.push_str(&format!("  - {}\n", c));
            }
        }

        if !self.parameters.is_empty() {
            s.push_str("Parameters:\n");
            for p in &self.parameters {
                s.push_str(&format!("  - {}\n", p));
            }
        }

        // Show convergence status
        s.push_str(&format!("\nIterations: {}/{}\n", self.iteration, self.max_iterations));
        if self.hit_iteration_limit {
            s.push_str("\n*** WARNING: Optimization did not converge ***\n");
            s.push_str("Stopped due to iteration limit. Results may not be optimal.\n");
            s.push_str("Consider increasing --max-optimize-iterations or narrowing parameter ranges.\n");
        } else if self.phase == OptimizerPhase::Complete {
            s.push_str("Status: Converged (completed all phases)\n");
        } else {
            s.push_str(&format!("Status: In progress ({:?})\n", self.phase));
        }

        if let Some(ref best) = self.best_result {
            s.push_str("\n=== Best Configuration ===\n");
            s.push_str(&format!("Config: {}\n", best.config));

            // Show primary goal metric
            let primary_metric = self.objectives.primary_metric();
            s.push_str(&format!("{}: {:.4}\n", primary_metric, best.objective_value));

            // Show all objective goal metrics
            if self.objectives.goals.len() > 1 {
                s.push_str("\nObjective metrics:\n");
                for goal in &self.objectives.goals {
                    if let Some(value) = best.metrics.get(&goal.metric) {
                        let unit = match goal.metric {
                            Metric::Qps => " req/s",
                            Metric::Recall | Metric::ErrorRate => "",
                            _ => " ms",
                        };
                        s.push_str(&format!("  {}: {:.4}{}\n", goal.metric, value, unit));
                    }
                }
            }

            s.push_str("\nAll metrics:\n");

            // Sort metrics for consistent output
            let mut metrics: Vec<_> = best.metrics.iter().collect();
            metrics.sort_by_key(|(m, _)| m.name());
            for (metric, value) in metrics {
                let unit = match metric {
                    Metric::Qps => " req/s",
                    Metric::Recall | Metric::ErrorRate => "",
                    _ => " ms",
                };
                s.push_str(&format!("  {}: {:.4}{}\n", metric, value, unit));
            }
        } else {
            s.push_str("\nNo configuration meeting all constraints was found.\n");
        }

        // Show recent history
        s.push_str("\n=== Measurement History ===\n");
        let start = if self.history.len() > 20 { self.history.len() - 20 } else { 0 };
        for (i, m) in self.history.iter().skip(start).enumerate() {
            let idx = start + i + 1;
            let status = if m.constraints_met { "OK" } else { "FAIL" };
            s.push_str(&format!(
                "{:3}. {} | {}={:.2} | {}\n",
                idx,
                m.config,
                self.objective.metric(),
                m.objective_value,
                status
            ));
        }

        s
    }
}

// ============================================================================
// Legacy compatibility (for old API)
// ============================================================================

/// Legacy Constraints type - kept for backward compatibility
#[derive(Debug, Clone)]
pub struct Constraints {
    pub min_recall: f64,
    pub max_p99_ms: Option<f64>,
    pub target_qps: Option<u64>,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            min_recall: 0.95,
            max_p99_ms: Some(100.0),
            target_qps: None,
        }
    }
}

impl Constraints {
    /// Convert to new Constraint list
    pub fn to_constraints(&self) -> Vec<Constraint> {
        let mut constraints = Vec::new();
        if self.min_recall > 0.0 {
            constraints.push(Constraint::gte(Metric::Recall, self.min_recall));
        }
        if let Some(max_p99) = self.max_p99_ms {
            constraints.push(Constraint::lte(Metric::P99Ms, max_p99));
        }
        if let Some(target_qps) = self.target_qps {
            constraints.push(Constraint::gte(Metric::Qps, target_qps as f64));
        }
        constraints
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::RecallStats;
    use hdrhistogram::Histogram;
    use std::time::Duration;

    fn make_result(recall: f64, qps: f64, p99_ms: f64) -> BenchmarkResult {
        let mut histogram = Histogram::new_with_bounds(1, 3_600_000_000, 3).expect("histogram");
        histogram.record((p99_ms * 1000.0) as u64).ok();

        let mut recall_stats = RecallStats::new();
        recall_stats.record(recall);

        BenchmarkResult {
            test_name: "test".to_string(),
            total_requests: 1000,
            duration: Duration::from_secs(1),
            throughput: qps,
            histogram,
            recall_stats,
            error_count: 0,
            node_metrics: Vec::new(),
            keyspace_stats: crate::benchmark::KeyspaceStats::default(),
        }
    }

    #[test]
    fn test_constraint_parsing() {
        let c = Constraint::parse("recall:gt:0.95").unwrap();
        assert_eq!(c.metric, Metric::Recall);
        assert_eq!(c.op, CompareOp::Gt);
        assert!((c.value - 0.95).abs() < 1e-9);

        let c = Constraint::parse("p99_ms:lt:0.1").unwrap();
        assert_eq!(c.metric, Metric::P99Ms);
        assert_eq!(c.op, CompareOp::Lt);
        assert!((c.value - 0.1).abs() < 1e-9);

        let c = Constraint::parse("qps:gte:100000").unwrap();
        assert_eq!(c.metric, Metric::Qps);
        assert_eq!(c.op, CompareOp::Gte);
    }

    #[test]
    fn test_objective_parsing() {
        let o = Objective::parse("maximize:qps").unwrap();
        assert!(matches!(o, Objective::Maximize(Metric::Qps)));

        let o = Objective::parse("minimize:p99_ms").unwrap();
        assert!(matches!(o, Objective::Minimize(Metric::P99Ms)));
    }

    #[test]
    fn test_parameter_parsing() {
        let p = TunableParameter::parse("ef_search:10:500:10").unwrap();
        assert_eq!(p.param_type, ParameterType::EfSearch);
        assert_eq!(p.min, 10);
        assert_eq!(p.max, 500);
        assert_eq!(p.step, 10);
        assert_eq!(p.num_values(), 50);

        let p = TunableParameter::parse("clients:10:200:10").unwrap();
        assert_eq!(p.param_type, ParameterType::Clients);
    }

    #[test]
    fn test_optimizer_get_benchmark() {
        // Test for GET workload: maximize QPS with p99 < 0.5ms
        let mut opt = Optimizer::builder()
            .objective(Objective::Maximize(Metric::Qps))
            .constraint(Constraint::lt(Metric::P99Ms, 0.5))
            .parameter(TunableParameter::clients(10, 100, 10))
            .max_iterations(15)
            .build();

        assert_eq!(opt.phase(), OptimizerPhase::Feasibility);

        while let Some(config) = opt.next_config() {
            let clients = config.clients.unwrap_or(50);
            // Simulate: more clients = higher QPS but also higher latency
            let qps = clients as f64 * 1000.0;
            let p99_ms = 0.1 + (clients as f64 * 0.005);  // latency increases with clients
            let result = make_result(0.0, qps, p99_ms);
            opt.record_result(config, &result);
        }

        assert!(opt.best_result().is_some());
        let best = opt.best_result().unwrap();
        // Best should maximize QPS while keeping p99 < 0.5ms
        let best_p99 = *best.metrics.get(&Metric::P99Ms).unwrap();
        assert!(best_p99 < 0.5, "p99 {} should be < 0.5ms", best_p99);
    }

    #[test]
    fn test_optimizer_vector_search() {
        // Test for vector search: maximize QPS with recall > 0.95
        let mut opt = Optimizer::builder()
            .objective(Objective::Maximize(Metric::Qps))
            .constraint(Constraint::gt(Metric::Recall, 0.95))
            .parameter(TunableParameter::ef_search(10, 200, 10))
            .max_iterations(15)
            .build();

        while let Some(config) = opt.next_config() {
            let ef = config.ef_search.unwrap_or(100);
            // Simulate: higher ef = better recall but lower QPS
            let recall = 0.7 + (ef as f64 * 0.0015).min(0.29);  // caps at 0.99
            let qps = 20000.0 - ef as f64 * 50.0;
            let result = make_result(recall, qps, 1.0);
            opt.record_result(config, &result);
        }

        assert!(opt.best_result().is_some());
        let best = opt.best_result().unwrap();
        let recall = *best.metrics.get(&Metric::Recall).unwrap();
        assert!(recall > 0.95, "recall {} should be > 0.95", recall);
    }

    #[test]
    fn test_constraint_evaluation() {
        let result = make_result(0.97, 5000.0, 0.25);

        assert!(Constraint::gt(Metric::Recall, 0.95).is_satisfied(&result));
        assert!(!Constraint::lt(Metric::Recall, 0.95).is_satisfied(&result));
        assert!(Constraint::lt(Metric::P99Ms, 0.5).is_satisfied(&result));
        assert!(!Constraint::lt(Metric::P99Ms, 0.1).is_satisfied(&result));
    }

    #[test]
    fn test_multi_parameter() {
        // Test with multiple parameters
        let mut opt = Optimizer::builder()
            .objective(Objective::Maximize(Metric::Qps))
            .constraint(Constraint::lt(Metric::P99Ms, 1.0))
            .parameter(TunableParameter::clients(10, 50, 10))
            .parameter(TunableParameter::threads(1, 4, 1))
            .max_iterations(30)
            .build();

        while let Some(config) = opt.next_config() {
            let clients = config.clients.unwrap_or(30) as f64;
            let threads = config.threads.unwrap_or(2) as f64;
            let qps = clients * threads * 100.0;
            let p99_ms = 0.1 + (clients * 0.01) + (threads * 0.05);
            let result = make_result(0.0, qps, p99_ms);
            opt.record_result(config, &result);
        }

        assert!(opt.best_result().is_some());
    }
}
