use std::collections::{btree_map::Values, BTreeMap, VecDeque};
use std::iter::Rev;

use asceswap_math::Price;
use asceswap_types::{OrderHash, Side};

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

    pub(crate) fn hashes_by_priority(&self, side: Side) -> LevelHashesIter<'_> {
        match side {
            Side::Buy => LevelHashesIter::Desc(self.levels.values().rev()),
            Side::Sell => LevelHashesIter::Asc(self.levels.values()),
        }
    }
}

pub(crate) enum LevelHashesIter<'a> {
    Asc(Values<'a, Price, VecDeque<OrderHash>>),
    Desc(Rev<Values<'a, Price, VecDeque<OrderHash>>>),
}

impl<'a> Iterator for LevelHashesIter<'a> {
    type Item = &'a VecDeque<OrderHash>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Asc(iter) => iter.next(),
            Self::Desc(iter) => iter.next(),
        }
    }
}
