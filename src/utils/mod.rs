pub mod certification;
pub mod math;
pub mod mem_context;
#[cfg(test)]
pub mod test;

#[cfg(target_family = "wasm")]
use ic_cdk::print;

#[cfg(target_family = "wasm")]
#[inline]
pub fn isoprint(str: &str) {
    print(str)
}

#[cfg(not(target_family = "wasm"))]
#[inline]
pub fn isoprint(str: &str) {
    println!("{}", str)
}
