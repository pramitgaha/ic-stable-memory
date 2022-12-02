use crate::collections::certified_hash_map::node::SCertifiedHashMapNode;
use sha2::{Digest, Sha256};
use std::fmt::Debug;

pub type Sha256Digest = [u8; 32];

pub trait ToHashableBytes {
    type Out: AsRef<[u8]>;

    fn to_hashable_bytes(&self) -> Self::Out;
}

macro_rules! impl_for_primitive {
    ($ty:ty) => {
        impl ToHashableBytes for $ty {
            type Out = [u8; std::mem::size_of::<$ty>()];

            fn to_hashable_bytes(&self) -> Self::Out {
                self.to_le_bytes()
            }
        }
    };
}

impl_for_primitive!(u8);
impl_for_primitive!(u16);
impl_for_primitive!(u32);
impl_for_primitive!(u64);
impl_for_primitive!(u128);
impl_for_primitive!(usize);
impl_for_primitive!(i8);
impl_for_primitive!(i16);
impl_for_primitive!(i32);
impl_for_primitive!(i64);
impl_for_primitive!(i128);
impl_for_primitive!(f32);
impl_for_primitive!(f64);
impl_for_primitive!(isize);

pub const EMPTY_SHA256: Sha256Digest = [0u8; 32];

#[derive(Debug)]
pub enum MerkleKV<K, V> {
    Plain((K, V)),
    Pruned(Sha256Digest),
}

impl<K: ToHashableBytes, V: ToHashableBytes> MerkleKV<K, V> {
    pub fn calculate_entry_sha256(&self, hasher: &mut Sha256) -> Sha256Digest {
        match self {
            MerkleKV::Plain((k, v)) => SCertifiedHashMapNode::<K, V>::sha256_entry(k, v, hasher),
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub enum MerkleChild {
    Pruned(Sha256Digest),
    Hole,
}

impl MerkleChild {
    pub fn unwrap(self) -> Sha256Digest {
        match self {
            MerkleChild::Pruned(d) => d,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub struct MerkleNode<K, V> {
    key_value: MerkleKV<K, V>,
    left_child: MerkleChild,
    right_child: MerkleChild,
}

impl<K, V> MerkleNode<K, V> {
    pub fn new(
        key_value: MerkleKV<K, V>,
        left_child: MerkleChild,
        right_child: MerkleChild,
    ) -> Self {
        Self {
            key_value,
            left_child,
            right_child,
        }
    }
}

// TODO: support multi-witnesses
#[derive(Debug)]
pub struct MerkleWitness<K, V> {
    pub tree: Vec<MerkleNode<K, V>>,
    pub additional_hashes: Vec<Option<Sha256Digest>>,
}

impl<K: ToHashableBytes, V: ToHashableBytes> MerkleWitness<K, V> {
    pub fn new(tree: Vec<MerkleNode<K, V>>, additional_hashes: Vec<Option<Sha256Digest>>) -> Self {
        Self {
            tree,
            additional_hashes,
        }
    }

    pub fn reconstruct(self) -> (MerkleKV<K, V>, Sha256Digest) {
        let mut branch = self.tree;
        let leaf = branch.remove(0);

        let mut hasher = Sha256::default();

        let kv_hash = leaf.key_value.calculate_entry_sha256(&mut hasher);

        let lc = leaf.left_child.unwrap();
        let rc = leaf.right_child.unwrap();

        hasher.update(kv_hash);
        hasher.update(lc);
        hasher.update(rc);

        let mut node_hash: Sha256Digest = hasher.finalize_reset().into();

        for node in branch {
            match node.key_value {
                MerkleKV::Pruned(vh) => hasher.update(vh),
                _ => unreachable!(),
            };

            match node.left_child {
                MerkleChild::Pruned(l_ch) => hasher.update(l_ch),
                MerkleChild::Hole => hasher.update(node_hash),
            };

            match node.right_child {
                MerkleChild::Pruned(r_ch) => hasher.update(r_ch),
                MerkleChild::Hole => hasher.update(node_hash),
            }

            node_hash = hasher.finalize_reset().into();
        }

        for add_opt in self.additional_hashes {
            match add_opt {
                Some(add) => hasher.update(add),
                None => hasher.update(node_hash),
            }
        }

        (leaf.key_value, hasher.finalize_reset().into())
    }
}
