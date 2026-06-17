use uuid::Uuid;

const BASE30: &[u8] = b"123456789ABCDEFGHKMNPRSTUVWXYZ";

pub fn id_16() -> String {
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();

    // Convert all 16 bytes of UUID into a single large number (u128)
    let mut num: u128 = 0;
    for &b in bytes {
        num = (num << 8) | b as u128;
    }

    // Encode the number into base30 using the custom alphabet
    let mut out = Vec::new();
    while num > 0 {
        out.push(BASE30[(num % 30) as usize]);
        num /= 30;
    }

    out.reverse();

    let mut s = String::from_utf8(out).unwrap();

    // Ensure the ID is exactly 16 characters:
    if s.len() < 16 {
        let pad = "J".repeat(16 - s.len());
        s = pad + &s;
    }
    s.truncate(16);

    format!("{}-{}-{}-{}", &s[0..4], &s[4..8], &s[8..12], &s[12..16])
}
