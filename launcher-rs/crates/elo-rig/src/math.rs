//! Vectors, quaternions and affine matrices in f64, the glTF way round:
//! quaternions are `[x, y, z, w]`, matrices are column-major.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V3(pub f64, pub f64, pub f64);

impl V3 {
    pub const ZERO: V3 = V3(0.0, 0.0, 0.0);
    pub fn from(a: &[f64]) -> V3 {
        V3(a[0], a[1], a[2])
    }
    pub fn add(self, o: V3) -> V3 {
        V3(self.0 + o.0, self.1 + o.1, self.2 + o.2)
    }
    pub fn sub(self, o: V3) -> V3 {
        V3(self.0 - o.0, self.1 - o.1, self.2 - o.2)
    }
    pub fn scale(self, k: f64) -> V3 {
        V3(self.0 * k, self.1 * k, self.2 * k)
    }
    pub fn dot(self, o: V3) -> f64 {
        self.0 * o.0 + self.1 * o.1 + self.2 * o.2
    }
    pub fn cross(self, o: V3) -> V3 {
        V3(
            self.1 * o.2 - self.2 * o.1,
            self.2 * o.0 - self.0 * o.2,
            self.0 * o.1 - self.1 * o.0,
        )
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    /// Unit length, or `None` for a vector too short to have a direction.
    pub fn norm(self) -> Option<V3> {
        let l = self.len();
        (l > 1e-9).then(|| self.scale(1.0 / l))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Q(pub f64, pub f64, pub f64, pub f64);

impl Q {
    pub const ID: Q = Q(0.0, 0.0, 0.0, 1.0);
    pub fn from(a: &[f64]) -> Q {
        Q(a[0], a[1], a[2], a[3]).norm()
    }
    pub fn norm(self) -> Q {
        let l = (self.0 * self.0 + self.1 * self.1 + self.2 * self.2 + self.3 * self.3).sqrt();
        if l < 1e-12 {
            Q::ID
        } else {
            Q(self.0 / l, self.1 / l, self.2 / l, self.3 / l)
        }
    }
    /// `self * o`: apply `o` first, then `self`.
    pub fn mul(self, o: Q) -> Q {
        let Q(ax, ay, az, aw) = self;
        let Q(bx, by, bz, bw) = o;
        Q(
            aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz,
        )
    }
    /// The inverse of a unit quaternion.
    pub fn inv(self) -> Q {
        Q(-self.0, -self.1, -self.2, self.3)
    }
    pub fn dot(self, o: Q) -> f64 {
        self.0 * o.0 + self.1 * o.1 + self.2 * o.2 + self.3 * o.3
    }
    pub fn neg(self) -> Q {
        Q(-self.0, -self.1, -self.2, -self.3)
    }
    pub fn rot(self, v: V3) -> V3 {
        let u = V3(self.0, self.1, self.2);
        let t = u.cross(v).scale(2.0);
        v.add(t.scale(self.3)).add(u.cross(t))
    }
    pub fn axis_angle(axis: V3, angle: f64) -> Q {
        let a = axis.norm().unwrap_or(V3(0.0, 1.0, 0.0));
        let s = (angle / 2.0).sin();
        Q(a.0 * s, a.1 * s, a.2 * s, (angle / 2.0).cos())
    }
    /// The shortest rotation taking direction `a` onto direction `b`.
    pub fn arc(a: V3, b: V3) -> Q {
        let (Some(a), Some(b)) = (a.norm(), b.norm()) else { return Q::ID };
        let d = a.dot(b);
        if d < -0.999_999 {
            // Opposite: any axis at right angles to `a`.
            let mut axis = V3(1.0, 0.0, 0.0).cross(a);
            if axis.len() < 1e-6 {
                axis = V3(0.0, 1.0, 0.0).cross(a);
            }
            return Q::axis_angle(axis, std::f64::consts::PI);
        }
        let c = a.cross(b);
        Q(c.0, c.1, c.2, 1.0 + d).norm()
    }
    pub fn slerp(self, mut b: Q, t: f64) -> Q {
        let mut d = self.dot(b);
        if d < 0.0 {
            b = b.neg();
            d = -d;
        }
        if d > 0.9995 {
            return Q(
                self.0 + (b.0 - self.0) * t,
                self.1 + (b.1 - self.1) * t,
                self.2 + (b.2 - self.2) * t,
                self.3 + (b.3 - self.3) * t,
            )
            .norm();
        }
        let th = d.acos();
        let s = th.sin();
        let wa = ((1.0 - t) * th).sin() / s;
        let wb = (t * th).sin() / s;
        Q(
            self.0 * wa + b.0 * wb,
            self.1 * wa + b.1 * wb,
            self.2 * wa + b.2 * wb,
            self.3 * wa + b.3 * wb,
        )
    }
    /// From the columns of an orthonormal basis.
    pub fn from_basis(x: V3, y: V3, z: V3) -> Q {
        let (m00, m11, m22) = (x.0, y.1, z.2);
        let tr = m00 + m11 + m22;
        if tr > 0.0 {
            let s = (tr + 1.0).sqrt() * 2.0;
            Q((y.2 - z.1) / s, (z.0 - x.2) / s, (x.1 - y.0) / s, 0.25 * s)
        } else if m00 > m11 && m00 > m22 {
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            Q(0.25 * s, (y.0 + x.1) / s, (z.0 + x.2) / s, (y.2 - z.1) / s)
        } else if m11 > m22 {
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            Q((y.0 + x.1) / s, 0.25 * s, (z.1 + y.2) / s, (z.0 - x.2) / s)
        } else {
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            Q((z.0 + x.2) / s, (z.1 + y.2) / s, 0.25 * s, (x.1 - y.0) / s)
        }
        .norm()
    }
}

/// A 4×4 affine matrix, column-major like glTF's `matrix`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M4(pub [f64; 16]);

impl M4 {
    pub const ID: M4 = M4([
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    pub fn trs(t: V3, r: Q, s: V3) -> M4 {
        let Q(x, y, z, w) = r;
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz, wx, wy, wz) = (x * y, x * z, y * z, w * x, w * y, w * z);
        M4([
            (1.0 - 2.0 * (yy + zz)) * s.0,
            2.0 * (xy + wz) * s.0,
            2.0 * (xz - wy) * s.0,
            0.0,
            2.0 * (xy - wz) * s.1,
            (1.0 - 2.0 * (xx + zz)) * s.1,
            2.0 * (yz + wx) * s.1,
            0.0,
            2.0 * (xz + wy) * s.2,
            2.0 * (yz - wx) * s.2,
            (1.0 - 2.0 * (xx + yy)) * s.2,
            0.0,
            t.0,
            t.1,
            t.2,
            1.0,
        ])
    }
    pub fn mul(&self, o: &M4) -> M4 {
        let (a, b) = (&self.0, &o.0);
        let mut m = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
            }
        }
        M4(m)
    }
    pub fn point(&self, p: V3) -> V3 {
        let m = &self.0;
        V3(
            m[0] * p.0 + m[4] * p.1 + m[8] * p.2 + m[12],
            m[1] * p.0 + m[5] * p.1 + m[9] * p.2 + m[13],
            m[2] * p.0 + m[6] * p.1 + m[10] * p.2 + m[14],
        )
    }
    pub fn origin(&self) -> V3 {
        V3(self.0[12], self.0[13], self.0[14])
    }
    /// The inverse of an affine matrix, `None` when it is singular.
    pub fn inv(&self) -> Option<M4> {
        let m = &self.0;
        // The 3×3 part, by cofactors; then the translation.
        let (a, b, c) = (m[0], m[4], m[8]);
        let (d, e, f) = (m[1], m[5], m[9]);
        let (g, h, i) = (m[2], m[6], m[10]);
        let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
        if det.abs() < 1e-14 {
            return None;
        }
        let k = 1.0 / det;
        let r = [
            (e * i - f * h) * k,
            (c * h - b * i) * k,
            (b * f - c * e) * k,
            (f * g - d * i) * k,
            (a * i - c * g) * k,
            (c * d - a * f) * k,
            (d * h - e * g) * k,
            (b * g - a * h) * k,
            (a * e - b * d) * k,
        ];
        // r is row-major: r[row*3 + col].
        let t = V3(m[12], m[13], m[14]);
        let tx = -(r[0] * t.0 + r[1] * t.1 + r[2] * t.2);
        let ty = -(r[3] * t.0 + r[4] * t.1 + r[5] * t.2);
        let tz = -(r[6] * t.0 + r[7] * t.1 + r[8] * t.2);
        Some(M4([
            r[0], r[3], r[6], 0.0, r[1], r[4], r[7], 0.0, r[2], r[5], r[8], 0.0, tx, ty, tz, 1.0,
        ]))
    }
    /// Translation, rotation and scale, for a matrix with no shear.
    pub fn decompose(&self) -> (V3, Q, V3) {
        let m = &self.0;
        let cx = V3(m[0], m[1], m[2]);
        let cy = V3(m[4], m[5], m[6]);
        let cz = V3(m[8], m[9], m[10]);
        let mut s = V3(cx.len(), cy.len(), cz.len());
        if cx.cross(cy).dot(cz) < 0.0 {
            s.0 = -s.0;
        }
        let safe = |v: V3, k: f64| if k.abs() < 1e-12 { v } else { v.scale(1.0 / k) };
        let r = Q::from_basis(safe(cx, s.0), safe(cy, s.1), safe(cz, s.2));
        (self.origin(), r, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Q, b: Q) -> bool {
        a.dot(b).abs() > 1.0 - 1e-9
    }

    #[test]
    fn arc_turns_a_onto_b() {
        let a = V3(1.0, 0.0, 0.0);
        let b = V3(0.0, -0.6, 0.8);
        let r = Q::arc(a, b).rot(a);
        assert!(r.sub(b).len() < 1e-9);
        let back = Q::arc(a, a.scale(-1.0)).rot(a);
        assert!(back.add(a).len() < 1e-9);
    }

    #[test]
    fn matrix_round_trips_trs() {
        let r = Q::axis_angle(V3(0.3, 1.0, -0.2), 1.1);
        let m = M4::trs(V3(1.0, 2.0, 3.0), r, V3(2.0, 2.0, 2.0));
        let (t, q, s) = m.decompose();
        assert!(t.sub(V3(1.0, 2.0, 3.0)).len() < 1e-9);
        assert!(close(q, r));
        assert!(s.sub(V3(2.0, 2.0, 2.0)).len() < 1e-9);
        let back = m.mul(&m.inv().unwrap());
        for (x, y) in back.0.iter().zip(M4::ID.0.iter()) {
            assert!((x - y).abs() < 1e-9);
        }
    }

    #[test]
    fn matrix_rotation_agrees_with_quaternion() {
        let r = Q::axis_angle(V3(1.0, 1.0, 0.0), 0.7);
        let p = V3(0.2, -1.0, 4.0);
        let a = M4::trs(V3::ZERO, r, V3(1.0, 1.0, 1.0)).point(p);
        assert!(a.sub(r.rot(p)).len() < 1e-9);
        let b = Q::from_basis(r.rot(V3(1.0, 0.0, 0.0)), r.rot(V3(0.0, 1.0, 0.0)), r.rot(V3(0.0, 0.0, 1.0)));
        assert!(close(b, r));
    }
}
