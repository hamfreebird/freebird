//! Stardust FFI 的 ID 分配与校验（readme.md §10.4）。
//!
//! `#[stardust(id = N)]` 直接指定 Stardust 函数 ID；省略时从函数名用
//! SipHash-2-4 派生，并映射到 `1..=999`，因为 Stardust 的函数名就是源码中的
//! 前导空格数，过大的 ID 会生成不可接受的巨量空格。

use crate::frontend::ast::{AttributeArg, FnDecl};
use crate::hir::types::HirFnDecl;

/// Stardust 函数 ID 的上限：ID 会变成生成源码中的前导空格数。
pub const MAX_STARDUST_ID: u64 = 4096;

/// 默认 ID 的取值上界（闭区间 1..=999）。
const DEFAULT_ID_MODULUS: u64 = 999;

/// 读取函数的 `#[stardust(id = N)]`；没有该属性时返回 `None`。
pub fn explicit_stardust_id_ast(f: &FnDecl) -> Option<u64> {
    f.attrs.iter().find_map(|attr| {
        if attr.name != "stardust" {
            return None;
        }
        attr.args.iter().find_map(|arg| match arg {
            AttributeArg::KeyValue(key, value) if key == "id" => value.parse::<u64>().ok(),
            _ => None,
        })
    })
}

/// 读取 HIR 函数的 `#[stardust(id = N)]`。
pub fn explicit_stardust_id(f: &HirFnDecl) -> Option<u64> {
    f.attrs.iter().find_map(|attr| {
        if attr.name != "stardust" {
            return None;
        }
        attr.args.iter().find_map(|arg| match arg {
            AttributeArg::KeyValue(key, value) if key == "id" => value.parse::<u64>().ok(),
            _ => None,
        })
    })
}

/// 解析 extern 函数的 Stardust ID：显式属性优先，否则从函数名派生。
pub fn stardust_id_for_fn(f: &HirFnDecl) -> u64 {
    explicit_stardust_id(f).unwrap_or_else(|| default_stardust_id(&f.name))
}

/// 从函数名派生默认 ID（SipHash-2-4，key = 0）。
pub fn default_stardust_id(name: &str) -> u64 {
    let hash = siphash24(name.as_bytes(), 0, 0);
    1 + hash % DEFAULT_ID_MODULUS
}

/// 校验 ID 是否可用于生成 Stardust 源码。
pub fn validate_stardust_id(id: u64, name: &str) -> Result<(), String> {
    if id > MAX_STARDUST_ID {
        return Err(format!(
            "stardust id {id} for extern function `{name}` is too large (max {MAX_STARDUST_ID}); \
             Stardust function IDs become leading spaces in generated source"
        ));
    }
    Ok(())
}

/// SipHash-2-4（key 由两个 u64 组成）。
///
/// 规范只要求“从函数名使用 SipHash-2-4 派生默认 ID”；这里采用标准
/// SipHash-2-4 实现，保证同一函数名在不同构建中稳定。
pub fn siphash24(data: &[u8], k0: u64, k1: u64) -> u64 {
    let mut v0 = 0x736f6d6570736575 ^ k0;
    let mut v1 = 0x646f72616e646f6d ^ k1;
    let mut v2 = 0x6c7967656e657261 ^ k0;
    let mut v3 = 0x7465646279746573 ^ k1;

    let mut chunks = data.chunks_exact(8);
    for chunk in &mut chunks {
        let m = u64::from_le_bytes(chunk.try_into().expect("8-byte chunk"));
        v3 ^= m;
        sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
        sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= m;
    }

    let rest = chunks.remainder();
    let mut last = (data.len() as u64) << 56;
    for (i, &byte) in rest.iter().enumerate() {
        last |= (byte as u64) << (8 * i);
    }
    v3 ^= last;
    sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
    sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= last;

    v2 ^= 0xff;
    sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
    sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
    sip_round(&mut v0, &mut v1, &mut v2, &mut v3);
    sip_round(&mut v0, &mut v1, &mut v2, &mut v3);

    v0 ^ v1 ^ v2 ^ v3
}

fn sip_round(v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64) {
    *v0 = v0.wrapping_add(*v1);
    *v1 = v1.rotate_left(13);
    *v1 ^= *v0;
    *v0 = v0.rotate_left(32);
    *v2 = v2.wrapping_add(*v3);
    *v3 = v3.rotate_left(16);
    *v3 ^= *v2;
    *v0 = v0.wrapping_add(*v3);
    *v3 = v3.rotate_left(21);
    *v3 ^= *v0;
    *v2 = v2.wrapping_add(*v1);
    *v1 = v1.rotate_left(17);
    *v1 ^= *v2;
    *v2 = v2.rotate_left(32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siphash24_matches_reference_vectors() {
        // SipHash-2-4 参考向量：key = 0x0706050403020100/0x0f0e0d0c0b0a0908，
        // 输入为 0x00..0x0e（15 字节）。
        let key0 = 0x0706050403020100u64;
        let key1 = 0x0f0e0d0c0b0a0908u64;
        let data: Vec<u8> = (0u8..15).collect();
        assert_eq!(siphash24(&data, key0, key1), 0xa129ca6149be45e5);
        // 空输入、key = 0 的稳定回归值。
        assert_eq!(siphash24(b"", 0, 0), 0x1e924b9d737700d7);
    }

    #[test]
    fn default_ids_are_stable_and_in_range() {
        let a = default_stardust_id("sd_add");
        let b = default_stardust_id("sd_add");
        assert_eq!(a, b);
        assert!((1..=999).contains(&a));
        assert_ne!(a, default_stardust_id("sd_print_char"));
    }
}
