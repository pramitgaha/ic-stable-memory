#[cfg(test)]
mod btree_map_benchmark {
    use crate::collections::btree_map::SBTreeMap;
    use crate::primitive::s_box::SBox;
    use crate::{init_allocator, measure, stable};
    use std::collections::BTreeMap;

    const ITERATIONS: usize = 100_00;

    #[test]
    #[ignore]
    fn body_indirect() {
        {
            let mut classic_btree_map = BTreeMap::new();

            measure!("Classic btree map insert", ITERATIONS, {
                for i in 0..ITERATIONS {
                    classic_btree_map.insert(i, String::from("Some short string"));
                }
            });

            measure!("Classic btree map search", ITERATIONS, {
                for i in 0..ITERATIONS {
                    classic_btree_map.get(&i).unwrap();
                }
            });

            measure!("Classic btree map remove", ITERATIONS, {
                for i in 0..ITERATIONS {
                    classic_btree_map.remove(&i).unwrap();
                }
            });
        }

        {
            stable::clear();
            stable::grow(1).unwrap();
            init_allocator(0);

            let mut stable_btree_map = SBTreeMap::new();

            measure!("Stable btree map insert", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_btree_map.insert(i, SBox::new(String::from("Some short string")));
                }
            });

            measure!("Stable btree map search", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_btree_map.get_copy(&i).unwrap();
                }
            });

            measure!("Stable btree map remove", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_btree_map.remove(&i).unwrap();
                }
            });
        }
    }

    #[test]
    #[ignore]
    fn body_direct() {
        {
            let mut classic_btree_map = BTreeMap::new();

            measure!("Classic btree map insert", ITERATIONS, {
                for i in 0..ITERATIONS {
                    classic_btree_map.insert(i, i);
                }
            });

            measure!("Classic btree map search", ITERATIONS, {
                for i in 0..ITERATIONS {
                    classic_btree_map.get(&i).unwrap();
                }
            });

            measure!("Classic btree map remove", ITERATIONS, {
                for i in 0..ITERATIONS {
                    classic_btree_map.remove(&i).unwrap();
                }
            });
        }

        {
            stable::clear();
            stable::grow(1).unwrap();
            init_allocator(0);

            let mut stable_btree_map = SBTreeMap::new();

            measure!("Stable btree map insert", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_btree_map.insert(i, i);
                }
            });

            measure!("Stable btree map search", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_btree_map.get_copy(&i).unwrap();
                }
            });

            measure!("Stable btree map remove", ITERATIONS, {
                for i in 0..ITERATIONS {
                    stable_btree_map.remove(&i).unwrap();
                }
            });
        }
    }

    #[test]
    #[ignore]
    fn cmp_with_new() {
        {
            let mut classic_btree_map = BTreeMap::new();

            measure!("Classic btree map insert", ITERATIONS, {
                for i in 2..ITERATIONS / 2 {
                    classic_btree_map.insert(ITERATIONS / 2 + i, i);
                    classic_btree_map.insert(ITERATIONS / 2 - i, i);
                }
            });

            measure!("Classic btree map search", ITERATIONS, {
                for i in 2..ITERATIONS / 2 {
                    classic_btree_map.get(&(ITERATIONS / 2 + i)).unwrap();
                    classic_btree_map.get(&(ITERATIONS / 2 - i)).unwrap();
                }
            });

            measure!("Classic btree map remove", ITERATIONS, {
                for i in 2..ITERATIONS / 2 {
                    classic_btree_map.remove(&(ITERATIONS / 2 + i)).unwrap();
                    classic_btree_map.remove(&(ITERATIONS / 2 - i)).unwrap();
                }
            });
        }

        {
            stable::clear();
            stable::grow(1).unwrap();
            init_allocator(0);

            let mut stable_btree_map = SBTreeMap::new();

            measure!("Stable btree map insert", ITERATIONS, {
                for i in 2..ITERATIONS / 2 {
                    stable_btree_map.insert(ITERATIONS / 2 + i, i);
                    stable_btree_map.insert(ITERATIONS / 2 - i, i);
                }
            });

            measure!("Stable btree map search", ITERATIONS, {
                for i in 2..ITERATIONS / 2 {
                    stable_btree_map.get_copy(&(ITERATIONS / 2 + i)).unwrap();
                    stable_btree_map.get_copy(&(ITERATIONS / 2 - i)).unwrap();
                }
            });

            measure!("Stable btree map remove", ITERATIONS, {
                for i in 2..ITERATIONS / 2 {
                    stable_btree_map.remove(&(ITERATIONS / 2 + i)).unwrap();
                    stable_btree_map.remove(&(ITERATIONS / 2 - i)).unwrap();
                }
            });
        }
    }
}
