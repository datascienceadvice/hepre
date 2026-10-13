//! Детерминированный генератор псевдослучайных чисел для воспроизводимого
//! синтеза когорты.
//!
//! Используется `xoshiro256**` (Blackman & Vigna, public domain) с
//! инициализацией состояния через `SplitMix64`. Обе конструкции оперируют
//! только целочисленной арифметикой `u64`, поэтому последовательность
//! **побитово одинакова на любой платформе**.
//!
//! Преобразование в `f64` берёт старшие 53 бита (`>> 11`), что даёт ровно
//! представимые значения в `[0, 1)`, и нормальный генератор использует
//! [`crate::detmath`] вместо платформенного `libm`.

use crate::detmath;

/// Потоковый генератор с явным воспроизводимым состоянием.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
    /// Кэш второго нормального отклонения (метод Марсальи «полярный»).
    spare_normal: Option<f64>,
}

/// Шаг `SplitMix64` — используется только для разворачивания семени в состояние.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    /// Создаёт генератор из 64-битного семени.
    ///
    /// Состояние разворачивается `SplitMix64`, поэтому принимается любое
    /// семя, включая нулевое (в отличие от голого `xoshiro`, где нулевое
    /// состояние вырождено).
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        let mut s = [0u64; 4];
        for slot in s.iter_mut() {
            *slot = splitmix64(&mut sm);
        }
        Self {
            s,
            spare_normal: None,
        }
    }

    /// Создаёт независимый подпоток по имени: одно и то же имя при одном и
    /// том же семени всегда даёт один и тот же поток.
    ///
    /// Нужно, чтобы разные части протокола (калибровка распространённости,
    /// генерация ковариат, генерация меток) не «съедали» значения друг у
    /// друга и оставались воспроизводимыми при изменении порядка вызовов.
    pub fn substream(seed: u64, name: &str) -> Self {
        // FNV-1a по имени потока → сдвиг семени.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in name.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        Self::new(seed ^ h)
    }

    /// Следующее 64-битное значение (`xoshiro256**`).
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;

        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);

        result
    }

    /// Равномерное значение в `[0, 1)` с 53 битами мантиссы.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        // 2^53 — точная константа, деление детерминировано.
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Равномерное значение в `[lo, hi)`.
    #[inline]
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// Случайное целое в `[0, n)`.
    ///
    /// Отбраковка значений из «хвоста», чтобы распределение было строго
    /// равномерным (иначе mod даёт смещение).
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0)");
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % n;
            }
        }
    }

    /// Бернуллиевская величина с вероятностью успеха `p`.
    #[inline]
    pub fn bernoulli(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }

    /// Стандартное нормальное отклонение (полярный метод Марсальи).
    ///
    /// Использует детерминированные `ln` и `sqrt`, поэтому результат
    /// платформо-независим.
    pub fn next_normal(&mut self) -> f64 {
        if let Some(v) = self.spare_normal.take() {
            return v;
        }
        loop {
            let u = 2.0 * self.next_f64() - 1.0;
            let v = 2.0 * self.next_f64() - 1.0;
            let s = u * u + v * v;
            if s > 0.0 && s < 1.0 {
                let factor = detmath::sqrt(-2.0 * detmath::ln(s) / s);
                self.spare_normal = Some(v * factor);
                return u * factor;
            }
        }
    }

    /// Нормальное отклонение с заданными средним и стандартным отклонением.
    #[inline]
    pub fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        mean + sd * self.next_normal()
    }

    /// Перемешивание Фишера—Йетса (воспроизводимое).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        if items.len() < 2 {
            return;
        }
        for i in (1..items.len()).rev() {
            let j = self.below((i + 1) as u64) as usize;
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_identical_stream() {
        let mut a = Rng::new(20260907);
        let mut b = Rng::new(20260907);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_diverges() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let sa: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let sb: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(sa, sb);
    }

    #[test]
    fn substreams_are_independent_and_stable() {
        let mut a = Rng::substream(7, "covariates");
        let mut b = Rng::substream(7, "labels");
        let sa: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let sb: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_ne!(sa, sb);

        let mut c = Rng::substream(7, "covariates");
        let sc: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        assert_eq!(sa, sc, "подпоток должен быть воспроизводим");
    }

    #[test]
    fn uniform_moments_are_reasonable() {
        let mut r = Rng::new(42);
        let n = 200_000;
        let mut sum = 0.0;
        let mut sum_sq = 0.0;
        let mut buckets = [0usize; 10];
        for _ in 0..n {
            let v = r.next_f64();
            assert!((0.0..1.0).contains(&v));
            sum += v;
            sum_sq += v * v;
            buckets[(v * 10.0) as usize] += 1;
        }
        let mean = sum / n as f64;
        let var = sum_sq / n as f64 - mean * mean;
        assert!((mean - 0.5).abs() < 0.005, "среднее {mean}");
        assert!((var - 1.0 / 12.0).abs() < 0.002, "дисперсия {var}");
        // Каждый из 10 интервалов должен получить ~n/10 значений (±5%).
        for (i, c) in buckets.iter().enumerate() {
            let expected = n as f64 / 10.0;
            assert!(
                (*c as f64 - expected).abs() < expected * 0.05,
                "интервал {i}: {c}"
            );
        }
    }

    #[test]
    fn normal_moments_are_reasonable() {
        let mut r = Rng::new(2024);
        let n = 200_000;
        let mut sum = 0.0;
        let mut sum_sq = 0.0;
        for _ in 0..n {
            let v = r.next_normal();
            sum += v;
            sum_sq += v * v;
        }
        let mean = sum / n as f64;
        let var = sum_sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.02, "среднее {mean}");
        assert!((var - 1.0).abs() < 0.03, "дисперсия {var}");
    }

    #[test]
    fn shuffle_is_a_permutation_and_deterministic() {
        let mut a: Vec<u32> = (0..100).collect();
        let mut b: Vec<u32> = (0..100).collect();
        Rng::new(5).shuffle(&mut a);
        Rng::new(5).shuffle(&mut b);
        assert_eq!(a, b, "перемешивание должно быть воспроизводимым");
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..100).collect::<Vec<u32>>());
        assert_ne!(a, sorted, "порядок должен измениться");
    }
}
