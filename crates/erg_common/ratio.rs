use core::fmt;
use std::cmp::Ordering::{self, Equal, Greater, Less};
use std::fmt::Display;
use std::ops::Neg;

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct Ratio {
    numer: i64,
    denom: i64,
}

impl Ratio {
    pub const fn new(numer: i64, denom: i64) -> Self {
        if numer == 0 {
            return Self::zero();
        }
        if denom == 0 {
            panic!("Zero is an invalid denominator!");
        }

        let gcd = gcd(numer, denom);
        let (numer, denom) = (numer / gcd, denom / gcd);
        if denom < 0 {
            Self {
                numer: -numer,
                denom: -denom,
            }
        } else {
            Self { numer, denom }
        }
    }

    #[inline]
    pub const fn zero() -> Self {
        Self { numer: 0, denom: 1 }
    }

    #[inline]
    pub fn one() -> Self {
        Self { numer: 1, denom: 1 }
    }

    const EPSILON: f64 = 1e-10;
    const MAX_DENOM: i64 = 10_000_000_000;

    /// This algorithm is O(log n) and finds the rational number with the smallest denominator
    /// within the given precision (EPSILON).
    #[inline]
    pub fn float_new(f: f64) -> Self {
        if f == 0.0 {
            return Self::zero();
        }
        let neg = f < 0.0;
        let f_abs = f.abs();
        let mut x = f_abs;

        let (mut p_prev2, mut p_prev1) = (0i64, 1i64);
        let (mut q_prev2, mut q_prev1) = (1i64, 0i64);

        loop {
            let a = x.floor() as i64;

            let p = a.checked_mul(p_prev1).and_then(|v| v.checked_add(p_prev2));
            let q = a.checked_mul(q_prev1).and_then(|v| v.checked_add(q_prev2));

            let (p, q) = match (p, q) {
                (Some(p), Some(q)) if q > 0 && q <= Self::MAX_DENOM => (p, q),
                _ => break,
            };

            let approx = p as f64 / q as f64;
            if (approx - f_abs).abs() < Self::EPSILON {
                return Self::new(if neg { -p } else { p }, q);
            }

            p_prev2 = p_prev1;
            p_prev1 = p;
            q_prev2 = q_prev1;
            q_prev1 = q;

            let frac = x - a as f64;
            if frac < 1e-15 {
                break;
            }
            x = 1.0 / frac;
        }

        if q_prev1 > 0 {
            Self::new(if neg { -p_prev1 } else { p_prev1 }, q_prev1)
        } else {
            Self::new(if neg { -p_prev2 } else { p_prev2 }, q_prev2.max(1))
        }
    }

    #[inline]
    pub fn to_float(self) -> f64 {
        self.numer as f64 / self.denom as f64
    }

    #[inline]
    pub fn to_int(self) -> i64 {
        self.numer / self.denom
    }

    #[inline]
    pub fn denom(&self) -> i64 {
        self.denom
    }

    #[inline]
    pub fn numer(&self) -> i64 {
        self.numer
    }

    #[inline]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        [self.numer.to_le_bytes(), self.denom.to_le_bytes()].concat()
    }

    /// Checked addition. Returns `None` on overflow.
    #[inline]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        let lcm = checked_lcm(self.denom, rhs.denom)?;
        // Use i128 for intermediate calculations to prevent overflow
        let l_numer = (self.numer as i128) * (lcm as i128 / self.denom as i128);
        let r_numer = (rhs.numer as i128) * (lcm as i128 / rhs.denom as i128);
        let numer = l_numer.checked_add(r_numer)?;
        // Check if result fits in i64
        if numer > i64::MAX as i128 || numer < i64::MIN as i128 {
            return None;
        }
        Some(Self::new(numer as i64, lcm))
    }

    /// Checked subtraction. Returns `None` on overflow.
    #[inline]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        let lcm = checked_lcm(self.denom, rhs.denom)?;
        let l_numer = (self.numer as i128) * (lcm as i128 / self.denom as i128);
        let r_numer = (rhs.numer as i128) * (lcm as i128 / rhs.denom as i128);
        let numer = l_numer.checked_sub(r_numer)?;
        if numer > i64::MAX as i128 || numer < i64::MIN as i128 {
            return None;
        }
        Some(Self::new(numer as i64, lcm))
    }

    /// Checked multiplication. Returns `None` on overflow.
    #[inline]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        if self.numer == 0 || rhs.numer == 0 {
            return Some(Self::zero());
        }
        let ac = gcd(self.numer, rhs.denom);
        let bd = gcd(rhs.numer, self.denom);
        let numer = (self.numer / ac).checked_mul(rhs.numer / bd)?;
        let denom = (self.denom / bd).checked_mul(rhs.denom / ac)?;
        Some(Self::new(numer, denom))
    }

    /// Checked division. Returns `None` on overflow or division by zero.
    #[inline]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        if rhs.numer == 0 {
            return None;
        }
        if self.numer == 0 {
            return Some(Self::zero());
        }
        let ac = gcd(self.numer, rhs.numer);
        let bd = gcd(self.denom, rhs.denom);
        let numer = (self.numer / ac).checked_mul(rhs.denom / bd)?;
        let denom = (self.denom / bd).checked_mul(rhs.numer / ac)?;
        Some(Self::new(numer, denom))
    }

    /// Checked remainder. Returns `None` on overflow or division by zero.
    #[inline]
    pub fn checked_rem(self, rhs: Self) -> Option<Self> {
        if rhs.numer == 0 {
            return None;
        }
        if self == rhs {
            return Some(Self::zero());
        } else if rhs == Self::one() {
            return Some(self);
        }
        let common_denom = gcd(self.denom, rhs.denom);
        let rhs_factor = rhs.denom / common_denom;
        let self_factor = self.denom / common_denom;
        // Use i128 to prevent overflow
        let l = (self.numer as i128) * (rhs_factor as i128);
        let r = (rhs.numer as i128) * (self_factor as i128);
        let numer = l % r;
        if numer > i64::MAX as i128 || numer < i64::MIN as i128 {
            return None;
        }
        let denom = self.denom.checked_mul(rhs_factor)?;
        Some(Self::new(numer as i64, denom))
    }
}

impl Neg for Ratio {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self::new(-self.numer, self.denom)
    }
}

impl PartialOrd for Ratio {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.denom == other.denom {
            return self.numer.partial_cmp(&other.numer);
        }

        if self.numer == other.numer {
            if self.numer == 0 {
                return Some(Equal);
            }
            let ordering = other.denom.cmp(&self.denom);
            return if self.numer > 0 {
                Some(ordering)
            } else {
                Some(ordering.reverse())
            };
        }

        // Use i128 to prevent overflow
        let left = self.numer as i128 * other.denom as i128;
        let right = self.denom as i128 * other.numer as i128;

        left.partial_cmp(&right)
    }

    #[inline]
    fn lt(&self, other: &Self) -> bool {
        matches!(self.partial_cmp(other), Some(Less))
    }

    #[inline]
    fn le(&self, other: &Self) -> bool {
        matches!(self.partial_cmp(other), Some(Less | Equal))
    }

    #[inline]
    fn gt(&self, other: &Self) -> bool {
        matches!(self.partial_cmp(other), Some(Greater))
    }

    #[inline]
    fn ge(&self, other: &Self) -> bool {
        matches!(self.partial_cmp(other), Some(Greater | Equal))
    }
}

impl Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.numer, self.denom)
    }
}

/// Binary GCD (Stein's Algorithm)
const fn gcd(mut u: i64, mut v: i64) -> i64 {
    if u < 0 {
        u = -u;
    }
    if v < 0 {
        v = -v;
    }

    if u == 0 {
        return v;
    }
    if v == 0 {
        return u;
    }

    let shift = (u | v).trailing_zeros();
    u >>= u.trailing_zeros();

    loop {
        v >>= v.trailing_zeros();
        if u > v {
            let tmp = u;
            u = v;
            v = tmp;
        }
        v -= u;
        if v == 0 {
            break;
        }
    }

    u << shift
}

fn checked_lcm(x: i64, y: i64) -> Option<i64> {
    if x == 0 || y == 0 {
        return Some(0);
    }
    (x / gcd(x, y)).checked_mul(y).map(|v| v.abs())
}

#[cfg(test)]
mod test {
    use super::Ratio;
    use crate::ratio::{checked_lcm, gcd};

    #[test]
    fn test_gcd() {
        assert_eq!(gcd(1, 1), 1);
        assert_eq!(gcd(54, 24), 6);
        assert_eq!(gcd(111, 30), 3);
        assert_eq!(gcd(239520000, 1293400200), 600);

        assert_eq!(gcd(0, 0), 0);
        assert_eq!(gcd(0, 5), 5);
        assert_eq!(gcd(5, 0), 5);

        assert_eq!(gcd(-1, -1), 1);
        assert_eq!(gcd(-5, -7), 1);
        assert_eq!(gcd(-54, 24), 6);
        assert_eq!(gcd(54, -24), 6);
        assert_eq!(gcd(-54, -24), 6);
        assert_eq!(gcd(-1, 1), gcd(1, -1));

        assert_eq!(gcd(i64::MAX, i64::MAX), i64::MAX);
        assert_eq!(gcd(0, i64::MAX), gcd(i64::MAX, 0));
    }

    #[test]
    fn test_checked_lcm() {
        assert_eq!(checked_lcm(7, 13), Some(91));
        assert_eq!(checked_lcm(0, 7), checked_lcm(7, 0));
        assert_eq!(checked_lcm(0, 0), Some(0));

        assert_eq!(checked_lcm(2, -3), Some(6));
        assert_eq!(checked_lcm(-2, 3), Some(6));
        assert_eq!(checked_lcm(-2, -3), Some(6));
        assert_eq!(checked_lcm(-5, 13), Some(65));
        assert_eq!(checked_lcm(-7, -13), Some(91));
        assert_eq!(checked_lcm(-1, -1), Some(1));

        assert_eq!(checked_lcm(i64::MAX, 1), Some(i64::MAX));
        assert_eq!(checked_lcm(1, i64::MAX), Some(i64::MAX));
        assert_eq!(checked_lcm(i64::MAX, 2), None);
    }

    #[test]
    fn test_new() {
        assert_eq!(Ratio::new(2, 4), Ratio::new(1, 2));
        assert_eq!(Ratio::new(i64::MAX, i64::MAX), Ratio::one());

        assert_eq!(Ratio::new(0, 12345), Ratio::zero());
        assert_eq!(Ratio::zero().numer(), 0);
        assert_eq!(Ratio::zero().denom(), 1);

        let r = Ratio::new(5, -3);
        assert_eq!(r.numer(), -5);
        assert_eq!(r.denom(), 3);

        let r = Ratio::new(-5, -3);
        assert_eq!(r.numer(), 5);
        assert_eq!(r.denom(), 3);

        let max = Ratio::new(i64::MAX, 1);
        assert_eq!(max.numer(), i64::MAX);
        assert_eq!(max.denom(), 1);
    }

    #[test]
    fn test_add() {
        assert_eq!(
            Ratio::new(1, 2).checked_add(Ratio::new(1, 3)),
            Some(Ratio::new(5, 6))
        );
        assert_eq!(
            Ratio::new(2, 1).checked_add(Ratio::new(10, 1)),
            Some(Ratio::new(12, 1))
        );
        assert_eq!(
            Ratio::new(1, 1).checked_add(Ratio::new(-1, 1)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(1, 1).checked_add(Ratio::new(1, -1)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(1, 1).checked_add(Ratio::new(-1, i64::MAX)),
            Some(Ratio::new(i64::MAX - 1, i64::MAX))
        );
        assert!(Ratio::new(i64::MAX / 2, 1)
            .checked_add(Ratio::new(1, 1))
            .is_some());
        assert!(Ratio::new(i64::MAX, 1).checked_add(Ratio::one()).is_none());
        assert!(Ratio::new(1, i64::MAX)
            .checked_add(Ratio::new(1, i64::MAX - 1))
            .is_none());
    }

    #[test]
    fn test_sub() {
        assert_eq!(
            Ratio::new(5, 6).checked_sub(Ratio::new(1, 3)),
            Some(Ratio::new(1, 2))
        );
        assert_eq!(
            Ratio::new(2, 1).checked_sub(Ratio::new(10, 1)),
            Some(Ratio::new(-8, 1))
        );
        assert_eq!(
            Ratio::new(1, 1).checked_sub(Ratio::new(2, 1)),
            Some(Ratio::new(-1, 1))
        );
        assert_eq!(
            Ratio::new(1, 1).checked_sub(Ratio::new(1, 1)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(i64::MAX, i64::MAX).checked_sub(Ratio::new(1, 1)),
            Some(Ratio::zero())
        );
    }

    #[test]
    fn test_mul() {
        assert_eq!(
            Ratio::new(2, 3).checked_mul(Ratio::new(3, 4)),
            Some(Ratio::new(1, 2))
        );
        assert_eq!(
            Ratio::new(2, 1).checked_mul(Ratio::new(10, 1)),
            Some(Ratio::new(20, 1))
        );
        assert_eq!(
            Ratio::new(3, 2).checked_mul(Ratio::new(2, 3)),
            Some(Ratio::one())
        );
        assert_eq!(
            Ratio::new(0, 1).checked_mul(Ratio::new(i64::MAX, 1)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(10, 1).checked_mul(Ratio::new(0, 1)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(i64::MAX, 2).checked_mul(Ratio::new(2, i64::MAX)),
            Some(Ratio::one())
        );
        assert!(Ratio::new(i64::MAX, 1)
            .checked_mul(Ratio::new(2, 1))
            .is_none());
    }

    #[test]
    fn test_div() {
        assert_eq!(
            Ratio::new(1, 2).checked_div(Ratio::new(1, 4)),
            Some(Ratio::new(2, 1))
        );
        assert_eq!(
            Ratio::new(2, 1).checked_div(Ratio::new(10, 1)),
            Some(Ratio::new(1, 5))
        );
        assert_eq!(
            Ratio::new(6, 1).checked_div(Ratio::new(2, 1)),
            Some(Ratio::new(3, 1))
        );
        assert_eq!(
            Ratio::new(80, 363).checked_div(Ratio::new(2, 5)),
            Some(Ratio::new(200, 363))
        );
        let a = Ratio::new(123, 456);
        assert_eq!(a.checked_div(Ratio::one()), Some(a));
        assert!(Ratio::new(1, 2).checked_div(Ratio::zero()).is_none());
        assert_eq!(
            Ratio::zero().checked_div(Ratio::new(5, 1)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(i64::MAX, i64::MIN + 1).checked_div(Ratio::new(i64::MAX, i64::MIN + 1)),
            Some(Ratio::one())
        );
    }

    #[test]
    fn test_rem() {
        assert_eq!(
            Ratio::new(5, 2).checked_rem(Ratio::new(5, 3)),
            Some(Ratio::new(5, 6))
        );
        assert_eq!(
            Ratio::new(7, 2).checked_rem(Ratio::new(2, 5)),
            Some(Ratio::new(3, 10))
        );
        assert_eq!(
            Ratio::new(2, 1).checked_rem(Ratio::new(10, 1)),
            Some(Ratio::new(2, 1))
        );
        assert_eq!(
            Ratio::new(3, 2).checked_rem(Ratio::new(3, 2)),
            Some(Ratio::zero())
        );
        assert_eq!(
            Ratio::new(i64::MAX, i64::MIN + 1).checked_rem(Ratio::new(i64::MAX, i64::MIN + 1)),
            Some(Ratio::zero())
        );
        assert!(Ratio::new(1, 2).checked_rem(Ratio::zero()).is_none());
        assert_eq!(
            Ratio::new(i64::MAX, 127).checked_rem(Ratio::new(i64::MAX, 7)),
            Some(Ratio::new(72624976668147841, 1))
        );
    }

    #[test]
    fn test_neg() {
        assert_eq!(-Ratio::new(3, 4), Ratio::new(-3, 4));
        assert_eq!(-(-Ratio::new(3, 4)), Ratio::new(3, 4));
        assert_eq!(-Ratio::new(-5, 7), Ratio::new(5, 7));
        assert_eq!(-Ratio::zero(), Ratio::zero());
    }

    #[test]
    fn test_cmp() {
        let a = Ratio::new(1, 2);
        let b = Ratio::new(1, 3);
        assert!(a > b);
        assert!(a >= a);
        assert!(a <= a);
        assert!(b >= b);
        assert!(b <= b);
        assert_eq!(Ratio::new(2, 4), Ratio::new(1, 2));
        let zero = Ratio::zero();
        let pos = Ratio::new(1, 2);
        let neg = Ratio::new(-1, 2);
        assert!(pos > zero);
        assert!(zero > neg);
        assert!(neg < zero);
        assert!(zero < pos);
        assert_eq!(-Ratio::zero(), Ratio::zero());
        assert!(Ratio::new(5, 2) > Ratio::new(5, 3));
        assert!(Ratio::new(-5, 2) < Ratio::new(-5, 3));
        assert!(Ratio::new(7, 10) > Ratio::new(7, 11));
        assert!(Ratio::new(i64::MAX, 1) > Ratio::new(i64::MAX - 1, 1));
        assert!(Ratio::new(1, i64::MAX) < Ratio::new(1, i64::MAX - 1));
        assert!(Ratio::new(i64::MAX, 2) > Ratio::new(i64::MAX, 3));
        assert!(Ratio::new(i64::MAX / 2, 1) > Ratio::new(i64::MAX / 3, 1));
        assert!(Ratio::new(1, 1000000000) > Ratio::new(1, 1000000001));
        assert!(Ratio::new(1000000000, 1) < Ratio::new(1000000001, 1));
        assert!(Ratio::new(i64::MAX, 1) > Ratio::new(i64::MIN + 1, 1));
    }

    #[test]
    fn test_float_new() {
        assert_eq!(Ratio::float_new(0.0), Ratio::zero());
        assert_eq!(Ratio::float_new(-0.0), Ratio::zero());
        assert_eq!(Ratio::float_new(1.0), Ratio::one());
        assert_eq!(Ratio::float_new(-1.0), Ratio::new(-1, 1));
        assert_eq!(Ratio::float_new(0.5), Ratio::new(1, 2));
        assert_eq!(Ratio::float_new(0.25), Ratio::new(1, 4));
        assert_eq!(Ratio::float_new(0.125), Ratio::new(1, 8));
        assert_eq!(Ratio::float_new(-0.5), Ratio::new(-1, 2));
        assert_eq!(Ratio::float_new(-2.5), Ratio::new(-5, 2));
        assert_eq!(Ratio::float_new(0.3), Ratio::new(3, 10));
        assert_eq!(Ratio::float_new(0.142857142857143), Ratio::new(1, 7));
        assert_eq!(Ratio::float_new(0.00100300902708124), Ratio::new(1, 997));
        assert_eq!(Ratio::float_new(1e-5), Ratio::new(1, 100000));
        assert_eq!(Ratio::float_new(1e-10), Ratio::new(1, 10000000000));
        assert_eq!(Ratio::float_new(100.0), Ratio::new(100, 1));
        assert_eq!(Ratio::float_new(1000000.0), Ratio::new(1000000, 1));
    }

    #[test]
    fn test_to_float() {
        assert_eq!(Ratio::new(1, 2).to_float(), 0.5);
        assert_eq!(Ratio::new(1, 4).to_float(), 0.25);
        assert_eq!(Ratio::new(-3, 4).to_float(), -0.75);
        assert_eq!(Ratio::zero().to_float(), 0.0);
        assert_eq!(Ratio::one().to_float(), 1.0);
    }

    #[test]
    fn test_to_int() {
        assert_eq!(Ratio::new(6, 2).to_int(), 3);
        assert_eq!(Ratio::new(7, 2).to_int(), 3);
        assert_eq!(Ratio::new(-7, 2).to_int(), -3);
        assert_eq!(Ratio::new(1, 3).to_int(), 0);
        assert_eq!(Ratio::zero().to_int(), 0);
    }

    #[test]
    fn test_to_le_bytes() {
        let r = Ratio::new(1, 2);
        assert_eq!(r.to_le_bytes().len(), 16); // 8 bytes for numer + 8 bytes for denom
        assert_eq!(Ratio::zero().to_le_bytes().len(), 16);
    }
}
