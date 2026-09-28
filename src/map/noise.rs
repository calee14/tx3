//! Small seeded value-noise implementation, so the map generator doesn't
//! need an extra dependency. Same seed = same noise on every machine.

pub struct ValueNoise {
    seed: u64,
}

impl ValueNoise {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Deterministic pseudo-random value in [0, 1) for a lattice point.
    fn lattice(&self, x: i64, y: i64) -> f32 {
        // SplitMix64-style hash of (seed, x, y).
        let mut h = self.seed
            ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= h >> 30;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        (h >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Smoothly interpolated noise at a continuous point, in [0, 1).
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let x0 = x.floor();
        let y0 = y.floor();
        let tx = smoothstep(x - x0);
        let ty = smoothstep(y - y0);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let a = self.lattice(ix, iy);
        let b = self.lattice(ix + 1, iy);
        let c = self.lattice(ix, iy + 1);
        let d = self.lattice(ix + 1, iy + 1);
        let top = a + (b - a) * tx;
        let bottom = c + (d - c) * tx;
        top + (bottom - top) * ty
    }

    /// Fractal noise: several octaves layered for natural-looking terrain.
    pub fn fbm(&self, x: f32, y: f32, octaves: u32) -> f32 {
        let mut total = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut norm = 0.0;
        for octave in 0..octaves {
            // Offset each octave so they don't line up on the lattice.
            let shift = octave as f32 * 17.31;
            total += self.sample(x * frequency + shift, y * frequency - shift) * amplitude;
            norm += amplitude;
            amplitude *= 0.5;
            frequency *= 2.0;
        }
        total / norm
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
