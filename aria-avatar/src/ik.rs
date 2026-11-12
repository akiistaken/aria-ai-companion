//! Analytic two-bone inverse kinematics, used for legs (feet planted on the ground) and arms
//! (a hand on the chin, the hips, a knee).
//!
//! Everything is in normalized model space, where a bone's rest rotation is identity. The solver
//! returns model-space rotations; the caller turns them into local ones with the [`crate::rig::Fk`]
//! frames.

use bevy::prelude::*;

/// A solved limb.
#[derive(Clone, Copy, Debug)]
pub struct Limb {
    /// Model-space rotation of the first bone (thigh, upper arm).
    pub upper: Quat,
    /// Model-space rotation of the second bone (shin, forearm).
    pub lower: Quat,
    /// Where the middle joint (knee, elbow) ends up.
    pub mid: Vec3,
}

/// Right-handed orthonormal basis with `dir` as its X axis and `normal` as its Y axis.
fn basis(dir: Vec3, normal: Vec3) -> Mat3 {
    Mat3::from_cols(dir, normal, dir.cross(normal))
}

/// A unit vector perpendicular to `dir`, as close to `hint` as possible.
fn perpendicular(dir: Vec3, hint: Vec3) -> Vec3 {
    (hint - dir * hint.dot(dir))
        .try_normalize()
        .unwrap_or_else(|| dir.any_orthonormal_vector())
}

/// Solves a two-bone chain.
///
/// * `root`: position of the first joint (hip, shoulder).
/// * `target`: where the end joint (ankle, wrist) should go.
/// * `to_mid`, `to_end`: the rest-pose offsets root→middle joint and middle→end joint. They do not
///   have to be collinear, so an A-pose or slightly bent rest pose is handled.
/// * `rest_pole`: the direction the middle joint moves toward when the limb bends, in the rest
///   pose (forward for a knee, backward for an elbow).
/// * `pole`: the direction the middle joint should point toward now.
pub fn two_bone(root: Vec3, target: Vec3, to_mid: Vec3, to_end: Vec3, rest_pole: Vec3, pole: Vec3) -> Limb {
    let l1 = to_mid.length().max(1e-4);
    let l2 = to_end.length().max(1e-4);
    let u1 = to_mid / l1;
    let u2 = to_end / l2;

    let reach = target - root;
    // Never fully straight (the joint would pop when it re-bends) and never folded flat.
    let dist = reach.length().clamp((l1 - l2).abs() + 1e-3, (l1 + l2) * 0.9995);
    let along = reach.try_normalize().unwrap_or(u1);

    // The bend plane contains `along` and the pole; `normal` is its normal and the hinge axis.
    let toward_pole = perpendicular(along, pole);
    let normal = along.cross(toward_pole);

    // Law of cosines for the angle at the root joint.
    let cos_root = ((l1 * l1 + dist * dist - l2 * l2) / (2.0 * l1 * dist)).clamp(-1.0, 1.0);
    let sin_root = (1.0 - cos_root * cos_root).sqrt();
    let mid = root + (along * cos_root + toward_pole * sin_root) * l1;
    let end = root + along * dist;

    let dir1 = (mid - root) / l1;
    let dir2 = (end - mid).try_normalize().unwrap_or(along);

    // Each bone turns from its rest direction and rest hinge axis to the solved ones.
    let rest_normal_1 = u1.cross(perpendicular(u1, rest_pole));
    let rest_normal_2 = u2.cross(perpendicular(u2, rest_pole));
    let upper = Quat::from_mat3(&(basis(dir1, normal) * basis(u1, rest_normal_1).transpose()));
    let lower = Quat::from_mat3(&(basis(dir2, normal) * basis(u2, rest_normal_2).transpose()));

    Limb {
        upper: upper.normalize(),
        lower: lower.normalize(),
        mid,
    }
}

/// The rotation that takes a rest direction pair (`rest_dir` with `rest_normal` perpendicular to
/// it) to a new pair. Used to aim hands and feet.
pub fn aim(rest_dir: Vec3, rest_normal: Vec3, dir: Vec3, normal: Vec3) -> Quat {
    let rest_dir = rest_dir.normalize_or(Vec3::X);
    let dir = dir.normalize_or(rest_dir);
    let from = basis(rest_dir, perpendicular(rest_dir, rest_normal));
    let to = basis(dir, perpendicular(dir, normal));
    Quat::from_mat3(&(to * from.transpose())).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    const THIGH: Vec3 = Vec3::new(0.0, -0.4, 0.0);
    const SHIN: Vec3 = Vec3::new(0.0, -0.4, 0.0);

    fn leg(root: Vec3, target: Vec3) -> Limb {
        two_bone(root, target, THIGH, SHIN, Vec3::Z, Vec3::Z)
    }

    #[test]
    fn the_solved_chain_reaches_the_target_with_rigid_bones() {
        let root = Vec3::new(0.1, 0.9, 0.0);
        for target in [
            Vec3::new(0.1, 0.2, 0.0),
            Vec3::new(0.3, 0.3, 0.2),
            Vec3::new(0.0, 0.5, 0.5),
            Vec3::new(-0.2, 0.4, -0.3),
        ] {
            let limb = leg(root, target);
            // The rotations reproduce the joint positions: bones keep their length.
            let mid = root + limb.upper * THIGH;
            let end = mid + limb.lower * SHIN;
            assert!(mid.distance(limb.mid) < 1e-4, "{target:?}");
            assert!(end.distance(target) < 1e-3, "{target:?}");
        }
    }

    #[test]
    fn a_knee_bends_toward_the_pole_and_an_elbow_away_from_it() {
        let limb = leg(Vec3::new(0.0, 0.9, 0.0), Vec3::new(0.0, 0.4, 0.0));
        assert!(limb.mid.z > 0.1, "the knee should come forward, got {:?}", limb.mid);

        // Left arm along +X, elbow pointing back.
        let arm = two_bone(
            Vec3::ZERO,
            Vec3::new(0.3, 0.0, 0.1),
            Vec3::new(0.25, 0.0, 0.0),
            Vec3::new(0.25, 0.0, 0.0),
            Vec3::NEG_Z,
            Vec3::NEG_Z,
        );
        assert!(arm.mid.z < 0.0, "the elbow should go back, got {:?}", arm.mid);
    }

    #[test]
    fn the_rest_pose_solves_to_identity() {
        let root = Vec3::new(0.0, 0.9, 0.0);
        // Just short of straight, straight down.
        let limb = leg(root, root + Vec3::new(0.0, -0.7996, 0.0));
        assert!(limb.upper.angle_between(Quat::IDENTITY) < 0.05);
        assert!(limb.lower.angle_between(Quat::IDENTITY) < 0.05);
    }

    #[test]
    fn out_of_reach_targets_are_pulled_in_not_broken() {
        let root = Vec3::new(0.0, 0.9, 0.0);
        let limb = leg(root, Vec3::new(0.0, -5.0, 0.0));
        assert!(limb.upper.is_finite() && limb.lower.is_finite());
        let end = root + limb.upper * THIGH + limb.lower * SHIN;
        assert!(end.distance(root) <= 0.8 && end.y < root.y, "pulled in, still pointing at it");
        // A target on top of the root is degenerate but must stay finite.
        let limb = leg(root, root);
        assert!(limb.upper.is_finite() && limb.lower.is_finite());
    }

    #[test]
    fn aim_turns_the_rest_pair_onto_the_new_pair() {
        // A left hand: fingers +X, palm facing down.
        let q = aim(Vec3::X, Vec3::NEG_Y, Vec3::Y, Vec3::Z);
        assert!((q * Vec3::X).distance(Vec3::Y) < 1e-4);
        assert!((q * Vec3::NEG_Y).distance(Vec3::Z) < 1e-4);
    }
}
