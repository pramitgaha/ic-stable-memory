use crate::mem::s_slice::Side;
use crate::primitive::StableAllocated;
use crate::utils::certification::{
    MerkleChild, MerkleKV, MerkleNode, Sha256Digest, ToHashableBytes, EMPTY_SHA256,
};
use crate::utils::phantom_data::SPhantomData;
use crate::{allocate, deallocate, SSlice};
use copy_as_bytes::traits::{AsBytes, SuperSized};
use sha2::{Digest, Sha256};
use speedy::{Context, LittleEndian, Readable, Reader, Writable, Writer};
use std::hash::{Hash, Hasher};
use zwohash::ZwoHasher;

// BY DEFAULT:
// LEN, CAPACITY: usize = 0
// NEXT: u64 = 0
// NODE_HASHES: [Sha256Digest; CAPACITY] = [zeroed(Sha256Digest); CAPACITY]
// ENTRY_HASHES: [Sha256Digest; CAPACITY] = [zeroed(Sha256Digest); CAPACITY]
// KEYS: [K; CAPACITY] = [zeroed(K); CAPACITY]
// VALUES: [V; CAPACITY] = [zeroed(V); CAPACITY]

const LEN_OFFSET: usize = 0;
const CAPACITY_OFFSET: usize = LEN_OFFSET + usize::SIZE;
const NEXT_OFFSET: usize = CAPACITY_OFFSET + usize::SIZE;
const NODE_HASHES_OFFSET: usize = NEXT_OFFSET + u64::SIZE;

#[inline]
pub const fn entry_hashes_offset(capacity: usize) -> usize {
    NODE_HASHES_OFFSET + Sha256Digest::SIZE * capacity
}

// TODO: check for overflow or use u64

#[inline]
pub const fn keys_offset(capacity: usize) -> usize {
    entry_hashes_offset(capacity) + Sha256Digest::SIZE * capacity
}

#[inline]
pub const fn values_offset<K: SuperSized>(capacity: usize) -> usize {
    keys_offset(capacity) + (K::SIZE + 1) * capacity
}

pub const DEFAULT_CAPACITY: usize = 7;

const EMPTY: u8 = 0;
const OCCUPIED: u8 = 255;

pub type KeyHash = usize;

// all for maximum cache-efficiency
// fixed-size, open addressing, linear probing, 3/4 load factor, non-lazy removal (https://stackoverflow.com/a/60709252/7171515)
pub struct SCertifiedHashMapNode<K, V> {
    pub(crate) table_ptr: u64,
    _marker_k: SPhantomData<K>,
    _marker_v: SPhantomData<V>,
}

impl<K, V> SCertifiedHashMapNode<K, V> {
    #[inline]
    pub unsafe fn from_ptr(table_ptr: u64) -> Self {
        Self {
            table_ptr,
            _marker_k: SPhantomData::default(),
            _marker_v: SPhantomData::default(),
        }
    }

    #[inline]
    pub unsafe fn copy(&self) -> Self {
        Self {
            table_ptr: self.table_ptr,
            _marker_k: SPhantomData::default(),
            _marker_v: SPhantomData::default(),
        }
    }

    #[inline]
    pub unsafe fn stable_drop_collection(&mut self) {
        let slice = SSlice::from_ptr(self.table_ptr, Side::Start).unwrap();
        deallocate(slice);
    }
}

impl<K: StableAllocated + ToHashableBytes + Eq + Hash, V: StableAllocated + ToHashableBytes>
    SCertifiedHashMapNode<K, V>
where
    [u8; K::SIZE]: Sized,
    [u8; V::SIZE]: Sized,
{
    #[inline]
    pub fn new(capacity: usize) -> Option<Self> {
        let bytes_capacity_opt = (1 + K::SIZE + V::SIZE)
            .checked_mul(capacity)
            .map(|it| it.checked_add(keys_offset(capacity)));

        if let Some(Some(size)) = bytes_capacity_opt {
            let table = allocate(size as usize);

            let zeroed = vec![0u8; size as usize];
            table.write_bytes(0, &zeroed);
            table.as_bytes_write(CAPACITY_OFFSET, capacity);

            return Some(Self {
                table_ptr: table.get_ptr(),
                _marker_k: SPhantomData::default(),
                _marker_v: SPhantomData::default(),
            });
        }

        None
    }

    pub fn insert(
        &mut self,
        mut key: K,
        mut value: V,
        capacity: usize,
        hasher: &mut Sha256,
    ) -> Result<(Option<V>, bool, usize, Sha256Digest), (K, V)> {
        let key_hash = Self::hash(&key);
        let entry_sha256 = Self::sha256_entry(&key, &value, hasher);

        let mut i = key_hash % capacity;

        loop {
            match self.read_key_at(i, true, capacity) {
                HashMapKey::Occupied(prev_key) => {
                    if prev_key.eq(&key) {
                        let mut prev_value = self.read_val_at(i, capacity);
                        prev_value.remove_from_stable();

                        value.move_to_stable();
                        self.write_val_at(i, value, capacity);

                        let (lc_sha256, rc_sha256) = self.read_children_node_hashes_of(i, capacity);

                        let mut node_sha256 =
                            Self::sha256_node(entry_sha256, lc_sha256, rc_sha256, hasher);

                        self.write_node_hash_at(i, node_sha256);
                        self.write_entry_hash_at(i, entry_sha256, capacity);

                        if i > 0 {
                            node_sha256 =
                                self.recalculate_merkle_tree(node_sha256, i, capacity, hasher);
                        }

                        return Ok((Some(prev_value), false, i, node_sha256));
                    } else {
                        i = (i + 1) % capacity;

                        continue;
                    }
                }
                HashMapKey::Empty => {
                    let len = self.len();
                    if self.is_full(len, capacity) {
                        return Err((key, value));
                    }

                    self.write_len(len + 1);

                    key.move_to_stable();
                    value.move_to_stable();

                    self.write_key_at(i, HashMapKey::Occupied(key), capacity);
                    self.write_val_at(i, value, capacity);

                    let (lc_sha256, rc_sha256) = self.read_children_node_hashes_of(i, capacity);

                    let mut node_sha256 =
                        Self::sha256_node(entry_sha256, lc_sha256, rc_sha256, hasher);

                    self.write_node_hash_at(i, node_sha256);
                    self.write_entry_hash_at(i, entry_sha256, capacity);

                    if i > 0 {
                        node_sha256 =
                            self.recalculate_merkle_tree(node_sha256, i, capacity, hasher);
                    }

                    return Ok((None, true, i, node_sha256));
                }
                _ => unreachable!(),
            }
        }
    }

    fn recalculate_merkle_tree(
        &mut self,
        mut node_sha256: Sha256Digest,
        mut i: usize,
        capacity: usize,
        hasher: &mut Sha256,
    ) -> Sha256Digest {
        let mut is_left = i % 2 == 1;

        while i > 0 {
            node_sha256 = if is_left {
                let r = if i + 1 < capacity {
                    self.read_node_hash_at(i + 1)
                } else {
                    EMPTY_SHA256
                };

                i /= 2;

                let entry_sha256 = self.get_entry_sha256_at(i, capacity);
                Self::sha256_node(entry_sha256, node_sha256, r, hasher)
            } else {
                let l = self.read_node_hash_at(i - 1);

                i = (i - 1) / 2;

                let entry_sha256 = self.get_entry_sha256_at(i, capacity);
                Self::sha256_node(entry_sha256, l, node_sha256, hasher)
            };

            self.write_node_hash_at(i, node_sha256);

            is_left = i % 2 == 1
        }

        node_sha256
    }

    pub fn remove_by_idx(
        &mut self,
        mut i: usize,
        capacity: usize,
        hasher: &mut Sha256,
    ) -> (V, Sha256Digest) {
        let prev_value = self.read_val_at(i, capacity);
        let mut j = i;

        let mut is = Vec::new();
        self.write_len(self.read_len() - 1);

        loop {
            j = (j + 1) % capacity;
            if j == i {
                break;
            }
            match self.read_key_at(j, true, capacity) {
                HashMapKey::Empty => break,
                HashMapKey::Occupied(next_key) => {
                    let key_hash = Self::hash(&next_key);
                    let k = key_hash % capacity;

                    if (j < i) ^ (k <= i) ^ (k > j) {
                        self.write_key_at(i, HashMapKey::Occupied(next_key), capacity);
                        self.write_val_at(i, self.read_val_at(j, capacity), capacity);
                        self.write_entry_hash_at(i, self.read_entry_hash_at(j, capacity), capacity);

                        is.push(i);

                        i = j;
                    }
                }
                _ => unreachable!(),
            }
        }

        self.write_key_at(i, HashMapKey::Empty, capacity);
        let (lc_sha256, rc_sha256) = self.read_children_node_hashes_of(i, capacity);

        let node_hash = Self::sha256_node(EMPTY_SHA256, lc_sha256, rc_sha256, hasher);
        self.write_node_hash_at(i, node_hash);

        let mut root_hash = if i > 0 {
            self.recalculate_merkle_tree(node_hash, i, capacity, hasher)
        } else {
            node_hash
        };

        // FIXME: PROBABLY INVALID
        // FIXME: we need a smarter function that will recalculate hashes of multiple keys at once
        for idx in is.into_iter().rev() {
            let (lc_sha256, rc_sha256) = self.read_children_node_hashes_of(idx, capacity);
            let entry_sha256 = self.get_entry_sha256_at(idx, capacity);

            let node_hash = Self::sha256_node(entry_sha256, lc_sha256, rc_sha256, hasher);
            self.write_node_hash_at(idx, node_hash);

            root_hash = self.recalculate_merkle_tree(node_hash, idx, capacity, hasher);
        }

        (prev_value, root_hash)
    }

    pub fn remove(
        &mut self,
        key: &K,
        capacity: usize,
        hasher: &mut Sha256,
    ) -> Option<(V, Sha256Digest)> {
        let (i, _) = self.find_inner_idx(key, capacity)?;

        Some(self.remove_by_idx(i, capacity, hasher))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.read_len()
    }

    #[inline]
    pub const fn is_full(&self, len: usize, capacity: usize) -> bool {
        if capacity < 12 {
            len == capacity * 3 / 4
        } else {
            len == capacity / 4 * 3
        }
    }

    pub fn find_inner_idx(&self, key: &K, capacity: usize) -> Option<(usize, K)> {
        let key_hash = Self::hash(key);
        let mut i = key_hash % capacity;

        loop {
            match self.read_key_at(i, true, capacity) {
                HashMapKey::Occupied(prev_key) => {
                    if prev_key.eq(key) {
                        return Some((i, prev_key));
                    } else {
                        i = (i + 1) % capacity;
                        continue;
                    }
                }
                HashMapKey::Empty => {
                    return None;
                }
                _ => unreachable!(),
            };
        }
    }

    pub fn witness_key(
        &self,
        key: &K,
        capacity: usize,
        hasher: &mut Sha256,
    ) -> Option<Vec<MerkleNode<K, V>>> {
        if let Some((mut idx, k)) = self.find_inner_idx(key, capacity) {
            let v = self.read_val_at(idx, capacity);
            let mut result = Vec::new();

            let (lc_hash, rc_hash) = self.read_children_node_hashes_of(idx, capacity);

            result.push(MerkleNode::new(
                MerkleKV::Plain((k, v)),
                MerkleChild::Pruned(lc_hash),
                MerkleChild::Pruned(rc_hash),
            ));

            if idx == 0 {
                return Some(result);
            }

            let mut is_left = idx % 2 == 1;

            while idx > 0 {
                if is_left {
                    let r = if idx + 1 < capacity {
                        self.read_node_hash_at(idx + 1)
                    } else {
                        EMPTY_SHA256
                    };

                    idx /= 2;

                    let entry_sha256 = self.get_entry_sha256_at(idx, capacity);

                    result.push(MerkleNode::new(
                        MerkleKV::Pruned(entry_sha256),
                        MerkleChild::Hole,
                        MerkleChild::Pruned(r),
                    ));
                } else {
                    let l = self.read_node_hash_at(idx - 1);

                    idx = (idx - 1) / 2;

                    let entry_sha256 = self.get_entry_sha256_at(idx, capacity);

                    result.push(MerkleNode::new(
                        MerkleKV::Pruned(entry_sha256),
                        MerkleChild::Pruned(l),
                        MerkleChild::Hole,
                    ));
                }

                is_left = idx % 2 == 1
            }

            Some(result)
        } else {
            None
        }
    }

    #[inline]
    pub fn read_len(&self) -> usize {
        SSlice::_as_bytes_read(self.table_ptr, LEN_OFFSET)
    }

    #[inline]
    pub fn read_capacity(&self) -> usize {
        SSlice::_as_bytes_read(self.table_ptr, CAPACITY_OFFSET)
    }

    #[inline]
    pub fn read_next(&self) -> u64 {
        SSlice::_as_bytes_read(self.table_ptr, NEXT_OFFSET)
    }

    fn read_key_at(&self, idx: usize, read_value: bool, capacity: usize) -> HashMapKey<K> {
        let mut key_flag = [0u8];
        let offset = keys_offset(capacity) + (1 + K::SIZE) * idx;

        SSlice::_read_bytes(self.table_ptr, offset, &mut key_flag);

        match key_flag[0] {
            EMPTY => HashMapKey::Empty,
            OCCUPIED => {
                if read_value {
                    let k = SSlice::_as_bytes_read(self.table_ptr, offset + 1);

                    HashMapKey::Occupied(k)
                } else {
                    HashMapKey::OccupiedNull
                }
            }
            _ => unreachable!(),
        }
    }

    fn get_entry_sha256_at(&self, idx: usize, capacity: usize) -> Sha256Digest {
        match self.read_key_at(idx, false, capacity) {
            HashMapKey::Empty => EMPTY_SHA256,
            HashMapKey::OccupiedNull => self.read_entry_hash_at(idx, capacity),
            _ => unreachable!(),
        }
    }

    #[inline]
    pub fn read_val_at(&self, idx: usize, capacity: usize) -> V {
        let offset = values_offset::<K>(capacity) + V::SIZE * idx;

        SSlice::_as_bytes_read(self.table_ptr, offset)
    }

    #[inline]
    pub fn read_node_hash_at(&self, idx: usize) -> Sha256Digest {
        let offset = NODE_HASHES_OFFSET + Sha256Digest::SIZE * idx;

        SSlice::_as_bytes_read(self.table_ptr, offset)
    }

    #[inline]
    pub fn read_entry_hash_at(&self, idx: usize, capacity: usize) -> Sha256Digest {
        let offset = entry_hashes_offset(capacity) + Sha256Digest::SIZE * idx;

        SSlice::_as_bytes_read(self.table_ptr, offset)
    }

    fn read_children_node_hashes_of(
        &self,
        idx: usize,
        capacity: usize,
    ) -> (Sha256Digest, Sha256Digest) {
        if idx >= (capacity - 1) / 2 {
            (EMPTY_SHA256, EMPTY_SHA256)
        } else {
            (
                self.read_node_hash_at((idx + 1) * 2 - 1),
                self.read_node_hash_at((idx + 1) * 2),
            )
        }
    }

    #[inline]
    pub fn read_root_hash(&self) -> Sha256Digest {
        self.read_node_hash_at(0)
    }

    #[inline]
    pub fn write_len(&mut self, len: usize) {
        SSlice::_as_bytes_write(self.table_ptr, LEN_OFFSET, len)
    }

    #[inline]
    pub fn write_capacity(&mut self, capacity: usize) {
        SSlice::_as_bytes_write(self.table_ptr, CAPACITY_OFFSET, capacity)
    }

    #[inline]
    pub fn write_next(&mut self, next: u64) {
        SSlice::_as_bytes_write(self.table_ptr, NEXT_OFFSET, next)
    }

    fn write_key_at(&mut self, idx: usize, key: HashMapKey<K>, capacity: usize) {
        let offset = keys_offset(capacity) + (1 + K::SIZE) * idx;

        let key_flag = match key {
            HashMapKey::Empty => [EMPTY],
            HashMapKey::Occupied(k) => {
                SSlice::_as_bytes_write(self.table_ptr, offset + 1, k);

                [OCCUPIED]
            }
            _ => unreachable!(),
        };

        SSlice::_write_bytes(self.table_ptr, offset, &key_flag);
    }

    #[inline]
    fn write_val_at(&mut self, idx: usize, val: V, capacity: usize) {
        let offset = values_offset::<K>(capacity) + V::SIZE * idx;

        SSlice::_as_bytes_write(self.table_ptr, offset, val);
    }

    #[inline]
    fn write_node_hash_at(&mut self, idx: usize, node_hash: Sha256Digest) {
        let offset = NODE_HASHES_OFFSET + Sha256Digest::SIZE * idx;

        SSlice::_as_bytes_write(self.table_ptr, offset, node_hash);
    }

    #[inline]
    fn write_entry_hash_at(&mut self, idx: usize, node_hash: Sha256Digest, capacity: usize) {
        let offset = entry_hashes_offset(capacity) + Sha256Digest::SIZE * idx;

        SSlice::_as_bytes_write(self.table_ptr, offset, node_hash);
    }

    pub fn debug_print(&self, capacity: usize) {
        print!(
            "Node({}, {}, {})[",
            self.read_len(),
            self.read_capacity(),
            self.read_next(),
        );
        for i in 0..capacity {
            let mut k_flag = [0u8];
            let mut k = [0u8; K::SIZE];
            let mut v = [0u8; V::SIZE];
            let mut hash = [0u8; Sha256Digest::SIZE];

            SSlice::_read_bytes(
                self.table_ptr,
                keys_offset(capacity) + (1 + K::SIZE) * i,
                &mut k_flag,
            );
            SSlice::_read_bytes(
                self.table_ptr,
                keys_offset(capacity) + (1 + K::SIZE) * i + 1,
                &mut k,
            );
            SSlice::_read_bytes(
                self.table_ptr,
                values_offset::<K>(capacity) + V::SIZE * i,
                &mut v,
            );
            SSlice::_read_bytes(
                self.table_ptr,
                NODE_HASHES_OFFSET + Sha256Digest::SIZE * i,
                &mut hash,
            );

            print!("(");

            match k_flag[0] {
                EMPTY => print!("<empty> = "),
                OCCUPIED => print!("<occupied> = "),
                _ => unreachable!(),
            };

            print!("k: {:?}, v: {:?}, h: /{:?}/)", k, v, hash);

            if i < capacity - 1 {
                print!(", ");
            }
        }
        println!("]");
    }

    pub fn hash(key: &K) -> KeyHash {
        let mut hasher = ZwoHasher::default();
        key.hash(&mut hasher);

        hasher.finish() as KeyHash
    }
}

impl<K: ToHashableBytes, V: ToHashableBytes> SCertifiedHashMapNode<K, V> {
    pub fn sha256_entry(k: &K, v: &V, hasher: &mut Sha256) -> Sha256Digest {
        hasher.update(k.to_hashable_bytes());
        hasher.update(v.to_hashable_bytes());

        hasher.finalize_reset().into()
    }

    pub fn sha256_node(
        entry_sha256: Sha256Digest,
        lc_sha256: Sha256Digest,
        rc_sha256: Sha256Digest,
        hasher: &mut Sha256,
    ) -> Sha256Digest {
        hasher.update(entry_sha256);
        hasher.update(lc_sha256);
        hasher.update(rc_sha256);

        hasher.finalize_reset().into()
    }
}

impl<K: StableAllocated + ToHashableBytes + Eq + Hash, V: StableAllocated + ToHashableBytes> Default
    for SCertifiedHashMapNode<K, V>
where
    [u8; K::SIZE]: Sized,
    [u8; V::SIZE]: Sized,
{
    #[inline]
    fn default() -> Self {
        unsafe { Self::new(DEFAULT_CAPACITY).unwrap_unchecked() }
    }
}

impl<'a, K, V> Readable<'a, LittleEndian> for SCertifiedHashMapNode<K, V> {
    fn read_from<R: Reader<'a, LittleEndian>>(
        reader: &mut R,
    ) -> Result<Self, <speedy::LittleEndian as Context>::Error> {
        let table_ptr = reader.read_u64()?;

        let it = Self {
            table_ptr,
            _marker_k: SPhantomData::default(),
            _marker_v: SPhantomData::default(),
        };

        Ok(it)
    }
}

impl<K, V> Writable<LittleEndian> for SCertifiedHashMapNode<K, V> {
    fn write_to<W: ?Sized + Writer<LittleEndian>>(
        &self,
        writer: &mut W,
    ) -> Result<(), <speedy::LittleEndian as Context>::Error> {
        writer.write_u64(self.table_ptr)
    }
}

enum HashMapKey<K> {
    Empty,
    Occupied(K),
    OccupiedNull,
}

impl<K> HashMapKey<K> {
    fn unwrap(self) -> K {
        match self {
            HashMapKey::Occupied(k) => k,
            _ => unreachable!(),
        }
    }
}

impl<K, V> SuperSized for SCertifiedHashMapNode<K, V> {
    const SIZE: usize = u64::SIZE;
}

impl<K, V> AsBytes for SCertifiedHashMapNode<K, V> {
    fn to_bytes(self) -> [u8; Self::SIZE] {
        self.table_ptr.to_bytes()
    }

    fn from_bytes(arr: [u8; Self::SIZE]) -> Self {
        let table_ptr = u64::from_bytes(arr);

        Self {
            table_ptr,
            _marker_k: SPhantomData::default(),
            _marker_v: SPhantomData::default(),
        }
    }
}

/*impl<K: StableAllocated + Eq + Hash, V: StableAllocated> StableAllocated for SHashMapNode<K, V>
where
    [u8; K::SIZE]: Sized,
    [u8; V::SIZE]: Sized,
{
    #[inline]
    fn move_to_stable(&mut self) {}

    #[inline]
    fn remove_from_stable(&mut self) {}

    unsafe fn stable_drop(mut self) {
        for (k, v) in self.iter() {
            k.stable_drop();
            v.stable_drop();
        }

        self.stable_drop_collection();
    }
}*/

#[cfg(test)]
mod tests {
    // TODO:
}
