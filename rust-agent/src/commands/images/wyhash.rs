//! Wyhash (final v4, seed 0) matching `Bun.hash.wyhash`, used to derive the
//! per-project blob cache file names the TypeScript runtime writes.
const SECRET: [u64; 4] =
	[0xa076_1d64_78bd_642f, 0xe703_7ed1_a0b4_28db, 0x8ebc_6af0_9c88_c6e3, 0x5899_65cc_7537_4cc3];

const fn mum(a: u64, b: u64) -> (u64, u64) {
	let r = (a as u128) * (b as u128);
	(r as u64, (r >> 64) as u64)
}

const fn mix(a: u64, b: u64) -> u64 {
	let (lo, hi) = mum(a, b);
	lo ^ hi
}

fn r8(input: &[u8], at: usize) -> u64 {
	u64::from_le_bytes(input[at..at + 8].try_into().expect("8 bytes"))
}

fn r4(input: &[u8], at: usize) -> u64 {
	u64::from(u32::from_le_bytes(input[at..at + 4].try_into().expect("4 bytes")))
}

pub fn hash(input: &[u8]) -> u64 {
	let len = input.len();
	let mut seed = mix(SECRET[0], SECRET[1]);
	let (a, b);
	if len <= 16 {
		if len >= 4 {
			let end = len - 4;
			let quarter = (len >> 3) << 2;
			a = (r4(input, 0) << 32) | r4(input, quarter);
			b = (r4(input, end) << 32) | r4(input, end - quarter);
		} else if len > 0 {
			a = (u64::from(input[0]) << 16)
				| (u64::from(input[len >> 1]) << 8)
				| u64::from(input[len - 1]);
			b = 0;
		} else {
			a = 0;
			b = 0;
		}
	} else {
		let mut i = 0;
		if len >= 48 {
			let (mut see1, mut see2) = (seed, seed);
			while i + 48 < len {
				seed = mix(r8(input, i) ^ SECRET[1], r8(input, i + 8) ^ seed);
				see1 = mix(r8(input, i + 16) ^ SECRET[2], r8(input, i + 24) ^ see1);
				see2 = mix(r8(input, i + 32) ^ SECRET[3], r8(input, i + 40) ^ see2);
				i += 48;
			}
			seed ^= see1 ^ see2;
		}
		while i + 16 < len {
			seed = mix(r8(input, i) ^ SECRET[1], r8(input, i + 8) ^ seed);
			i += 16;
		}
		a = r8(input, len - 16);
		b = r8(input, len - 8);
	}
	let (a, b) = mum(a ^ SECRET[1], b ^ seed);
	mix(a ^ SECRET[0] ^ len as u64, b ^ SECRET[1])
}
