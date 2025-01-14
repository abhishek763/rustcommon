// Copyright 2019-2020 Twitter, Inc.
// Licensed under the Apache License, Version 2.0
// http://www.apache.org/licenses/LICENSE-2.0

//! This library provides a thread safe token bucket ratelimitier

#![deny(clippy::all)]

use core::convert::TryFrom;
use rustcommon_time::AtomicDuration;
use rustcommon_time::AtomicInstant;
use rustcommon_time::Duration;
use rustcommon_time::Instant;

use rand_distr::{Distribution, Normal, Uniform};
use rustcommon_atomics::*;

/// A token bucket ratelimiter
pub struct Ratelimiter {
    available: AtomicU64,
    capacity: AtomicU64,
    quantum: AtomicU64,
    strategy: AtomicUsize,
    tick: AtomicDuration,
    next: AtomicInstant,
    normal: Normal<f64>,
    uniform: Uniform<f64>,
}

/// Refill strategies define how the token bucket is refilled. The different
/// strategies can be used to alter the character of the smoothness of the
/// interval between tokens becoming available
#[derive(Copy, Clone, Debug)]
pub enum Refill {
    /// Use a fixed tick interval resulting in a smooth ratelimit
    Smooth = 0,
    /// Use a uniform distribution for tick interval resulting in a ratelimit
    /// that varies from 2/3 to 3/2 times the specified rate. Resulting in an
    /// average ratelimit that matches the configured rate.
    Uniform = 1,
    /// Use a normal distribution for the tick interval centered on the duration
    /// matching that of the smooth refill strategy. The distribution used has
    /// a standard deviation of 2x the mean and results in an average ratelimit
    /// that matches the configured rate.
    Normal = 2,
}

impl TryFrom<usize> for Refill {
    type Error = ();

    fn try_from(v: usize) -> Result<Self, Self::Error> {
        match v {
            x if x == Refill::Smooth as usize => Ok(Refill::Smooth),
            x if x == Refill::Uniform as usize => Ok(Refill::Uniform),
            x if x == Refill::Normal as usize => Ok(Refill::Normal),
            _ => Err(()),
        }
    }
}

const SECOND: u64 = 1_000_000_000;

/// A token bucket ratelimiter
impl Ratelimiter {
    /// Create a new token bucket `Ratelimiter` which can hold up to `capacity`
    /// tokens. `quantum` tokens will be added to the bucket at `rate` times
    /// per second. The token bucket initially starts without any tokens, this
    /// ensures the rate does not start high initially.
    ///
    /// # Examples
    ///
    /// ```
    /// use rustcommon_ratelimiter::*;
    ///
    /// // ratelimit to 1/s with no burst capacity
    /// let ratelimiter = Ratelimiter::new(1, 1, 1);
    ///
    /// // ratelimit to 100/s with bursts up to 10
    /// let ratelimiter = Ratelimiter::new(10, 1, 100);
    /// ```
    pub fn new(capacity: u64, quantum: u64, rate: u64) -> Self {
        let tick = SECOND / (rate / quantum);
        Self {
            available: AtomicU64::default(),
            capacity: AtomicU64::new(capacity),
            quantum: AtomicU64::new(quantum),
            strategy: AtomicUsize::new(Refill::Smooth as usize),
            tick: AtomicDuration::from_nanos(tick),
            next: AtomicInstant::now(),
            normal: Normal::new(tick as f64, 2.0 * tick as f64).unwrap(),
            uniform: Uniform::new_inclusive(tick as f64 * 0.5, tick as f64 * 1.5),
        }
    }

    /// Changes the rate of the `Ratelimiter`. The new rate will be in effect on
    /// the next tick.
    pub fn set_rate(&self, rate: u64) {
        self.tick.store(
            Duration::from_nanos(SECOND / (rate / self.quantum.load(Ordering::Relaxed))),
            Ordering::Relaxed,
        );
    }

    /// Returns the current rate
    pub fn rate(&self) -> u64 {
        SECOND * self.quantum.load(Ordering::Relaxed)
            / self.tick.load(Ordering::Relaxed).as_nanos() as u64
    }

    /// Changes the refill strategy
    pub fn set_strategy(&self, strategy: Refill) {
        self.strategy.store(strategy as usize, Ordering::Relaxed)
    }

    // internal function to move the time forward
    fn tick(&self) {
        let now = Instant::now();
        let next = self.next.load(Ordering::Relaxed);
        if now >= next {
            let strategy = Refill::try_from(self.strategy.load(Ordering::Relaxed));
            let tick = match strategy {
                Ok(Refill::Smooth) => self.tick.load(Ordering::Relaxed).as_nanos() as u64,
                Ok(Refill::Uniform) => self.uniform.sample(&mut rand::thread_rng()) as u64,
                Ok(Refill::Normal) => self.normal.sample(&mut rand::thread_rng()) as u64,
                Err(_) => self.tick.load(Ordering::Relaxed).as_nanos() as u64,
            };
            if self
                .next
                .compare_exchange(
                    next,
                    next + Duration::from_nanos(tick),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                let quantum = self.quantum.load(Ordering::Relaxed);
                let capacity = self.capacity.load(Ordering::Relaxed);
                let available = self.available.load(Ordering::Relaxed);
                if available + quantum >= capacity {
                    let quantum = capacity - available;
                    self.available.fetch_add(quantum, Ordering::Relaxed);
                } else {
                    self.available.fetch_add(quantum, Ordering::Relaxed);
                }
            }
        }
    }

    /// Non-blocking wait for one token, returns an `Ok` if a token was
    /// successfully acquired, returns an `Err` if it would block.
    ///
    /// # Examples
    ///
    /// ```
    /// use rustcommon_ratelimiter::*;
    ///
    /// let ratelimiter = Ratelimiter::new(1, 1, 100);
    /// for i in 0..100 {
    ///     // do some work here
    ///     while ratelimiter.try_wait().is_err() {
    ///         // do some other work
    ///     }
    /// }
    /// ```
    pub fn try_wait(&self) -> Result<(), std::io::Error> {
        self.tick();
        if self.available.load(Ordering::Relaxed) > 0 {
            self.available.fetch_saturating_sub(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "no tokens in bucket",
            ))
        }
    }

    /// Blocking wait implemented as a busy loop. Returns only after a token is
    /// successfully acquired
    ///
    /// # Examples
    ///
    /// ```
    /// use rustcommon_ratelimiter::*;
    ///
    /// let ratelimiter = Ratelimiter::new(1, 1, 100);
    /// for i in 0..100 {
    ///     // do some work here
    ///     ratelimiter.wait();
    /// }
    /// ```
    pub fn wait(&self) {
        while self.try_wait().is_err() {}
    }
}
