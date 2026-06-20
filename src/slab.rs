use crate::book::RestingOrder;
use crate::types::Qty;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    pub(crate) order: RestingOrder,
    pub(crate) prev: Option<u32>,
    pub(crate) next: Option<u32>,
}

#[derive(Debug, Default)]
pub(crate) struct Slab {
    pub(crate) nodes: Vec<Node>,
    free: Vec<u32>,
}

impl Slab {
    pub(crate) fn alloc(&mut self, node: Node) -> u32 {
        match self.free.pop() {
            Some(slot) => {
                self.nodes[slot as usize] = node;
                slot
            }
            None => {
                let slot = self.nodes.len() as u32;
                self.nodes.push(node);
                slot
            }
        }
    }

    pub(crate) fn dealloc(&mut self, slot: u32) {
        self.free.push(slot);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PriceLevel {
    pub(crate) head: Option<u32>,
    pub(crate) tail: Option<u32>,
    pub(crate) total_qty: Qty,
}

impl PriceLevel {
    pub(crate) fn link_back(&mut self, slab: &mut Slab, slot: u32) {
        let prev = self.tail;
        {
            let node = &mut slab.nodes[slot as usize];
            node.prev = prev;
            node.next = None;
        }
        match prev {
            Some(p) => slab.nodes[p as usize].next = Some(slot),
            None => self.head = Some(slot),
        }
        self.tail = Some(slot);
    }

    pub(crate) fn unlink(&mut self, slab: &mut Slab, slot: u32) {
        let (prev, next) = {
            let node = &slab.nodes[slot as usize];
            (node.prev, node.next)
        };
        match prev {
            Some(p) => slab.nodes[p as usize].next = next,
            None => self.head = next,
        }
        match next {
            Some(n) => slab.nodes[n as usize].prev = prev,
            None => self.tail = prev,
        }
    }
}
