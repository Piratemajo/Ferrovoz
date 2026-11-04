//! Atenuación por distancia y paneo estéreo según la orientación del oyente.
//!
//! Coordenadas de Minecraft: +X este, +Y arriba, +Z sur. Yaw en grados:
//! 0 = mirando al sur (+Z), 90 = oeste (-X), 180 = norte, 270 = este.

#[derive(Debug, Clone, Copy)]
pub struct SpatialParams {
    /// Por debajo de esta distancia se oye a volumen completo.
    pub full_volume_distance: f32,
    /// A partir de esta distancia no se oye nada.
    pub max_distance: f32,
    /// Atenuación extra para sonidos detrás del oyente (1.0 = ninguna).
    pub behind_gain: f32,
}

impl Default for SpatialParams {
    fn default() -> Self {
        Self { full_volume_distance: 4.0, max_distance: 48.0, behind_gain: 0.8 }
    }
}

/// Ganancias (izq., der.) para un hablante en `rel` (hablante - oyente).
pub fn stereo_gains(rel: [f32; 3], listener_yaw_deg: f32, p: &SpatialParams) -> (f32, f32) {
    let dist = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
    let distance_gain = distance_gain(dist, p);
    if distance_gain <= 0.0 {
        return (0.0, 0.0);
    }

    let yaw = listener_yaw_deg.to_radians();
    let (sin, cos) = yaw.sin_cos();
    let forward = (-sin, cos);
    let right = (-cos, -sin);

    let horiz = (rel[0] * rel[0] + rel[2] * rel[2]).sqrt();
    let (pan, front) = if horiz < 0.5 {
        (0.0, 1.0) // encima, debajo o en el mismo sitio: centrado
    } else {
        let (x, z) = (rel[0] / horiz, rel[2] / horiz);
        (x * right.0 + z * right.1, x * forward.0 + z * forward.1)
    };

    // Paneo de potencia constante, limitado a +-85 % para que un lateral puro no
    // suene solo por un oído.
    let pan = pan.clamp(-1.0, 1.0) * 0.85;
    let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    let behind = if front < 0.0 { 1.0 + (p.behind_gain - 1.0) * (-front) } else { 1.0 };
    let g = distance_gain * behind * std::f32::consts::SQRT_2;
    (angle.cos() * g, angle.sin() * g)
}

pub fn distance_gain(dist: f32, p: &SpatialParams) -> f32 {
    if dist <= p.full_volume_distance {
        return 1.0;
    }
    if dist >= p.max_distance {
        return 0.0;
    }
    let t = 1.0 - (dist - p.full_volume_distance) / (p.max_distance - p.full_volume_distance);
    // Cae más rápido al principio.
    t * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: SpatialParams = SpatialParams { full_volume_distance: 4.0, max_distance: 48.0, behind_gain: 0.8 };

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn centered_when_in_front() {
        // Mirando al sur, hablante al sur.
        let (l, r) = stereo_gains([0.0, 0.0, 3.0], 0.0, &P);
        assert!(approx(l, r) && approx(l, 1.0), "{l} {r}");
    }

    #[test]
    fn east_is_on_the_left_when_facing_south() {
        let (l, r) = stereo_gains([3.0, 0.0, 0.0], 0.0, &P);
        assert!(l > 1.3 && r < l * 0.2, "{l} {r}");
    }

    #[test]
    fn turning_swaps_sides() {
        // Mirando al norte (180), el este queda a la derecha.
        let (l, r) = stereo_gains([3.0, 0.0, 0.0], 180.0, &P);
        assert!(r > 1.3 && l < r * 0.2, "{l} {r}");
    }

    #[test]
    fn behind_is_quieter() {
        let (lf, rf) = stereo_gains([0.0, 0.0, 3.0], 0.0, &P);
        let (lb, rb) = stereo_gains([0.0, 0.0, -3.0], 0.0, &P);
        assert!(lb + rb < lf + rf);
    }

    #[test]
    fn distance_falloff() {
        assert_eq!(distance_gain(2.0, &P), 1.0);
        assert_eq!(distance_gain(48.0, &P), 0.0);
        let mid = distance_gain(26.0, &P);
        assert!(mid > 0.0 && mid < 0.5);
        assert!(distance_gain(10.0, &P) > distance_gain(20.0, &P));
    }

    #[test]
    fn constant_power_when_turning() {
        // Hablante al este: a la izquierda (yaw 0), a la derecha (180) o delante (270).
        let p = |yaw| {
            let (l, r) = stereo_gains([5.0, 0.0, 0.0], yaw, &P);
            l * l + r * r
        };
        assert!(approx(p(0.0), p(180.0)) && approx(p(0.0), p(270.0)));
    }
}
