use anyhow::Result;

pub type Hash = [u8; 32];

#[must_use] 
pub fn hash_bytes(data: &[u8]) -> Hash {
    blake3::hash(data).into()
}

#[must_use] 
pub fn hash_to_hex(hash: &Hash) -> String {
    hex::encode(hash)
}

pub fn hex_to_hash(s: &str) -> Result<Hash> {
    let bytes = hex::decode(s)?;
    bytes.try_into().map_err(|_| anyhow::anyhow!("invalid hash length"))
}
