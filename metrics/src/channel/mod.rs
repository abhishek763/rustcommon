// Copyright 2020 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

use crate::entry::Entry;
use crate::outputs::ApproxOutput;
use crate::summary::SummaryStruct;
use crate::traits::*;
use crate::MetricsError;
use crate::Output;
use crate::Summary;
use rustcommon_time::Instant;

use crossbeam::atomic::AtomicCell;
use dashmap::DashSet;
use rustcommon_atomics::{Atomic, AtomicBool, Ordering};

/// Internal type which stores fields necessary to track a corresponding
/// statistic.
pub struct Channel<Value, Count>
where
    Value: crate::Value,
    Count: crate::Count,
    <Value as Atomic>::Primitive: Primitive,
    <Count as Atomic>::Primitive: Primitive,
    u64: From<<Value as Atomic>::Primitive> + From<<Count as Atomic>::Primitive>,
{
    refreshed: AtomicCell<Instant>,
    statistic: Entry<Value, Count>,
    empty: AtomicBool,
    reading: Value,
    summary: Option<SummaryStruct<Value, Count>>,
    outputs: DashSet<ApproxOutput>,
}

impl<Value, Count> Channel<Value, Count>
where
    Value: crate::Value,
    Count: crate::Count,
    <Value as Atomic>::Primitive: Primitive,
    <Count as Atomic>::Primitive: Primitive,
    u64: From<<Value as Atomic>::Primitive> + From<<Count as Atomic>::Primitive>,
{
    /// Creates an empty channel for a statistic.
    pub fn new(statistic: &dyn Statistic<Value, Count>) -> Self {
        let summary = statistic.summary().map(|v| v.build());
        Self {
            empty: AtomicBool::new(true),
            statistic: Entry::from(statistic),
            reading: Default::default(),
            refreshed: AtomicCell::new(Instant::now()),
            summary,
            outputs: Default::default(),
        }
    }

    /// Records a bucket value + count pair into the summary.
    pub fn record_bucket(
        &self,
        time: Instant,
        value: <Value as Atomic>::Primitive,
        count: <Count as Atomic>::Primitive,
    ) -> Result<(), MetricsError> {
        if let Some(summary) = &self.summary {
            summary.increment(time, value, count);
            Ok(())
        } else {
            Err(MetricsError::NoSummary)
        }
    }

    /// Updates a counter to a new value if the reading is newer than the stored
    /// reading.
    pub fn record_counter(&self, time: Instant, value: <Value as Atomic>::Primitive) {
        let t0 = self.refreshed.load();
        if time <= t0 {
            return;
        }
        if !self.empty.load(Ordering::Relaxed) {
            if let Some(summary) = &self.summary {
                self.refreshed.store(time);
                let v0 = self.reading.load(Ordering::Relaxed);
                let dt = time - t0;
                let dv = (value - v0).to_float();
                let rate = (dv
                    / (dt.as_secs() as f64 + dt.subsec_nanos() as f64 / 1_000_000_000.0))
                    .ceil();
                summary.increment(
                    time,
                    <Value as Atomic>::Primitive::from_float(rate),
                    1_u8.into(),
                );
            }
            self.reading.store(value, Ordering::Relaxed);
        } else {
            self.reading.store(value, Ordering::Relaxed);
            self.empty.store(false, Ordering::Relaxed);
            self.refreshed.store(time);
        }
    }

    /// Increment a counter by an amount
    pub fn increment_counter(&self, value: <Value as Atomic>::Primitive) {
        self.empty.store(false, Ordering::Relaxed);
        self.reading.fetch_add(value, Ordering::Relaxed);
    }

    /// Updates a gauge reading if the new value is newer than the stored value.
    pub fn record_gauge(&self, time: Instant, value: <Value as Atomic>::Primitive) {
        {
            let t0 = self.refreshed.load();
            if time <= t0 {
                return;
            }
        }
        if let Some(summary) = &self.summary {
            summary.increment(time, value, 1_u8.into());
        }
        self.reading.store(value, Ordering::Relaxed);
        self.empty.store(false, Ordering::Relaxed);
        self.refreshed.store(time);
    }

    /// Returns a percentile across stored readings/rates/...
    pub fn percentile(
        &self,
        percentile: f64,
    ) -> Result<<Value as Atomic>::Primitive, MetricsError> {
        if let Some(summary) = &self.summary {
            summary.percentile(percentile).map_err(MetricsError::from)
        } else {
            Err(MetricsError::NoSummary)
        }
    }

    /// Returns the main reading for the channel (eg: counter, gauge)
    pub fn reading(&self) -> Result<<Value as Atomic>::Primitive, MetricsError> {
        if !self.empty.load(Ordering::Relaxed) {
            Ok(self.reading.load(Ordering::Relaxed))
        } else {
            Err(MetricsError::Empty)
        }
    }

    /// Set a summary to be used for an existing channel
    pub fn set_summary(&mut self, summary: Summary<Value, Count>) {
        let summary = summary.build();
        self.summary = Some(summary);
    }

    /// Set a summary to be used for an existing channel
    pub fn add_summary(&mut self, summary: Summary<Value, Count>) {
        if self.summary.is_none() {
            self.set_summary(summary);
        }
    }

    pub fn statistic(&self) -> &dyn Statistic<Value, Count> {
        &self.statistic
    }

    pub fn outputs(&self) -> Vec<ApproxOutput> {
        let mut ret = Vec::new();
        for output in self.outputs.iter().map(|v| *v) {
            ret.push(output);
        }
        ret
    }

    pub fn add_output(&self, output: Output) {
        self.outputs.insert(ApproxOutput::from(output));
    }

    pub fn remove_output(&self, output: Output) {
        self.outputs.remove(&ApproxOutput::from(output));
    }
}
