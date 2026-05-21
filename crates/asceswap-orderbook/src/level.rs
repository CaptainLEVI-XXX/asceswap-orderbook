use std::collections::{BTreeMap, VecDeque};

use asceswap_math::Price;
use asceswap_types::OrderHash;

#[derive(Clone, Debug, Default)]
pub(crate) struct PriceLevelBook {
    pub(crate) levels: BTreeMap<Price, VecDeque<OrderHash>>,
}

impl PriceLevelBook {
    pub(crate) fn insert(&mut self, price: Price, hash: OrderHash) {
        self.levels.entry(price).or_default().push_back(hash);
    }

    pub(crate) fn remove(&mut self, price: Price, hash: OrderHash) {
        let mut remove_level = false;
        if let Some(level) = self.levels.get_mut(&price) {
            if let Some(index) = level.iter().position(|candidate| *candidate == hash) {
                level.remove(index);
            }
            remove_level = level.is_empty();
        }

        if remove_level {
            self.levels.remove(&price);
        }
    }
}
