use zene_llm::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageSnapshot {
    pub(crate) usage: TokenUsage,
    pub(crate) context_tokens: u32,
    pub(crate) context_window: u32,
    pub(crate) context_percent: u8,
    pub(crate) context_epoch: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UsageAccumulator {
    total: TokenUsage,
}

impl UsageAccumulator {
    pub(crate) fn reset(&mut self) {
        self.total = TokenUsage::default();
    }

    pub(crate) fn record(&mut self, usage: &TokenUsage) {
        self.total.accumulate(usage);
    }

    pub(crate) fn total(&self) -> &TokenUsage {
        &self.total
    }

    pub(crate) fn snapshot(
        &self,
        context_tokens: u32,
        context_window: u32,
        context_percent: u8,
        context_epoch: u64,
    ) -> UsageSnapshot {
        UsageSnapshot {
            usage: self.total,
            context_tokens,
            context_window,
            context_percent,
            context_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_sums_and_resets_usage() {
        let mut accumulator = UsageAccumulator::default();
        accumulator.record(&TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 2,
            total_tokens: 12,
            cached_tokens: Some(4),
        });
        accumulator.record(&TokenUsage {
            prompt_tokens: 3,
            completion_tokens: 1,
            total_tokens: 4,
            cached_tokens: Some(1),
        });
        assert_eq!(accumulator.total().prompt_tokens, 13);
        assert_eq!(accumulator.total().cached_tokens, Some(5));
        let snapshot = accumulator.snapshot(700, 1000, 70, 3);
        assert_eq!(snapshot.usage.prompt_tokens, 13);
        assert_eq!(snapshot.context_tokens, 700);
        accumulator.reset();
        assert_eq!(*accumulator.total(), TokenUsage::default());
    }
}
