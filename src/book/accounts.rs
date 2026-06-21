use std::collections::HashMap;

use crate::types::{AccountId, Qty, Side};

#[derive(Clone, Copy, Debug, Default)]
struct AccountState {
    net: i128,
    open_buy: Qty,
    open_sell: Qty,
}

#[derive(Debug, Default)]
pub(crate) struct AccountBook {
    accounts: HashMap<AccountId, AccountState>,
}

impl AccountBook {
    pub(crate) fn add_open(&mut self, owner: AccountId, side: Side, qty: Qty) {
        if owner == 0 || qty == 0 {
            return;
        }
        let state = self.accounts.entry(owner).or_default();
        match side {
            Side::Buy => state.open_buy += qty,
            Side::Sell => state.open_sell += qty,
        }
    }

    pub(crate) fn sub_open(&mut self, owner: AccountId, side: Side, qty: Qty) {
        if owner == 0 || qty == 0 {
            return;
        }
        if let Some(state) = self.accounts.get_mut(&owner) {
            match side {
                Side::Buy => state.open_buy = state.open_buy.saturating_sub(qty),
                Side::Sell => state.open_sell = state.open_sell.saturating_sub(qty),
            }
        }
    }

    pub(crate) fn fill(&mut self, owner: AccountId, side: Side, qty: Qty) {
        if owner == 0 || qty == 0 {
            return;
        }
        let state = self.accounts.entry(owner).or_default();
        match side {
            Side::Buy => state.net += qty as i128,
            Side::Sell => state.net -= qty as i128,
        }
    }

    pub(crate) fn net(&self, owner: AccountId) -> i128 {
        self.accounts.get(&owner).map_or(0, |s| s.net)
    }

    pub(crate) fn open(&self, owner: AccountId, side: Side) -> Qty {
        self.accounts.get(&owner).map_or(0, |s| match side {
            Side::Buy => s.open_buy,
            Side::Sell => s.open_sell,
        })
    }

    pub(crate) fn restore_net(&mut self, owner: AccountId, net: i128) {
        if owner == 0 || net == 0 {
            return;
        }
        self.accounts.entry(owner).or_default().net = net;
    }

    pub(crate) fn nets_sorted(&self) -> Vec<(AccountId, i128)> {
        let mut out: Vec<(AccountId, i128)> = self
            .accounts
            .iter()
            .filter(|(_, s)| s.net != 0)
            .map(|(id, s)| (*id, s.net))
            .collect();
        out.sort_by_key(|(id, _)| *id);
        out
    }
}
