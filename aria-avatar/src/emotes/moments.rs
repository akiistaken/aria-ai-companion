//! Idle moments: little things she does on her own when nothing is going on (the app picks one,
//! usually through the AI, to fit her mood). Each plays out on `ctx.e`, the time since it began:
//! a beginning, a middle she can hold, and a settle back toward standing.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use super::{fract, hand_to_face, hands_together, idle, neutral, pulse, sx, Act, Ctx};
use crate::face::Face;
use crate::pose::{hand_aim, Arm, HandShape, Landmarks, Pose};
use crate::rig::Side;
use crate::spring::{bump, smoothstep};

/// Both arms up over her head, fingers laced, up on her toes and leaning side to side.
pub fn stretch(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let e = ctx.e;
    let up = smoothstep(0.0, 0.7, e) * (1.0 - smoothstep(3.4, 4.3, e));
    let lean = (((e - 0.9).max(0.0)) * 1.4).sin() * smoothstep(0.8, 1.3, e) * up;
    act.pose.spine(-0.12 * up, 0.0, 0.16 * lean);
    act.pose.head(-0.18 * up, 0.0, 0.1 * lean);
    let mut raised = Pose::default();
    for side in Side::BOTH {
        raised.shrug(side, 0.2, 0.0);
        raised.arm(side, Arm { down: -1.35, swing: 0.0, yaw: 0.35, twist: -1.35, bend: 0.55, pronate: 0.0 });
        raised.wrist(side, 0.3, 0.0, 0.0);
        raised.fingers(side, HandShape::FLAT.tighten(0.35));
        act.pose.blend_bones(&raised, super::arm_bones(side), up);
        act.feet[side.index()].heel = 0.45 * up * (1.0 - lean.abs());
    }
    // Arms drop with a little shake-out as she comes down.
    let shake = bump((e - 3.4) / 1.2) * (e * 14.0).sin();
    for side in Side::BOTH {
        act.pose.wrist(side, 0.0, 0.15 * shake, 0.0);
    }
    act.face = Face { eyes_closed: 0.85 * up, aa: 0.35 * bump((e - 0.4) / 1.6), relaxed: 0.5, ..default() };
    act.attention = 0.1;
    act.walk_arms = 0.3;
    act
}

/// Looks around the desktop: head and body turning one way, then the other, a hand shading her
/// eyes for the far look.
pub fn look_around(ctx: &Ctx) -> Act {
    let mut act = idle(ctx);
    let (e, u, m) = (ctx.e, ctx.u(), &ctx.rig.metrics);
    // Left, hold, right, hold, back: a 5-second loop.
    let cycle = fract(e / 5.0) * 5.0;
    let look = smoothstep(0.2, 0.9, cycle) - 2.0 * smoothstep(2.0, 2.8, cycle) + smoothstep(4.0, 4.8, cycle);
    act.pose.spine(0.0, 0.18 * look, 0.0);
    act.pose.head(-0.06, 0.55 * look, 0.06 * look);
    act.gaze = Vec2::new(0.8 * look, 0.1);
    // Shading her eyes while looking far to the right.
    let shade = smoothstep(2.4, 2.9, cycle) * (1.0 - smoothstep(3.6, 4.0, cycle));
    if shade > 0.0 {
        let mut shading = act.pose.clone();
        hand_to_face(
            &mut shading,
            ctx,
            Side::Right,
            m.eye + Vec3::new(0.0, 0.07 * u, 0.06 * u),
            Vec3::new(0.95, 0.0, 0.25),
            Vec3::new(-0.6, -0.4, 0.4),
        );
        shading.fingers(Side::Right, HandShape::FLAT);
        act.pose.blend_bones(&shading, super::arm_bones(Side::Right), shade);
    }
    act.attention = 0.0;
    act.face = Face { surprised: 0.15, ..default() };
    act
}

/// Humming to herself: eyes half closed, a gentle sway, hands clasped behind her back.
pub fn hum(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (t, u) = (ctx.t, ctx.u());
    let sway = (t * 2.0).sin();
    act.pose.pelvis(0.0, 0.0, 0.05 * sway);
    act.pose.spine(0.03, 0.0, -0.05 * sway);
    act.pose.head(0.02, 0.0, 0.15 * sway);
    let lm = Landmarks::of(ctx.rig, &act.pose);
    for side in Side::BOTH {
        let target = lm.on(crate::rig::Bone::Hips, sx(side, 0.045 * u, 0.035 * u, -0.135 * u));
        let hand = hand_aim(side, sx(side, -0.8, -0.55, 0.0), Vec3::NEG_Z);
        act.pose.reach(ctx.rig, side, target, sx(side, 1.0, -0.15, -0.35), Some(hand));
        act.pose.fingers(side, HandShape::RELAXED);
    }
    // Rocking heel to toe on one foot.
    act.feet[Side::Right.index()].heel = 0.25 * pulse(t * 2.0);
    act.face = Face { relaxed: 0.7, eyes_closed: 0.55, ou: 0.18 + 0.1 * pulse(t * 6.3), ..default() };
    act.attention = 0.2;
    act.walk_arms = 0.4;
    act
}

/// A quick twirl on the spot, arms out, landing in a cute pose with her hands under her chin.
pub fn twirl(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (e, u) = (ctx.e, ctx.u());
    let turn = smoothstep(0.15, 1.05, e);
    act.spin = if turn >= 1.0 { 0.0 } else { TAU * turn };
    let spinning = bump(turn);
    let mut out = Pose::default();
    for side in Side::BOTH {
        out.arm(side, Arm { down: 0.35, swing: 0.1, yaw: 0.0, twist: -0.3, bend: 0.25, pronate: 0.0 });
        out.fingers(side, HandShape::OPEN);
        act.pose.blend_bones(&out, super::arm_bones(side), spinning);
    }
    act.hop = 0.04 * u * bump(e / 0.5);
    // Landing: hands together under the chin, head tilted, a little heel pop.
    let landed = smoothstep(1.0, 1.35, e);
    if landed > 0.0 {
        let mut cute = act.pose.clone();
        hands_together(&mut cute, ctx, 0.11, 0.13, 0.03, HandShape::RELAXED.tighten(0.2));
        cute.head(0.0, 0.0, 0.15);
        act.pose.blend(&cute, landed);
        act.feet[Side::Left.index()].heel = 0.5 * landed * bump((e - 1.0) / 1.2).max(0.3);
    }
    act.face = Face { happy: 0.9, ..default() };
    act.attention = 1.0 - spinning;
    act.walk_arms = 0.5;
    act
}

/// Tucks her hair behind her ear, then smooths it down, head tilted into the hand.
pub fn fix_hair(ctx: &Ctx) -> Act {
    let mut act = idle(ctx);
    let (e, u, m) = (ctx.e, ctx.u(), &ctx.rig.metrics);
    let side = Side::Left;
    let reach = smoothstep(0.0, 0.5, e) * (1.0 - smoothstep(2.6, 3.2, e));
    // Stroking down from the temple to behind the ear, twice.
    let stroke = 0.5 - 0.5 * ((e - 0.5).max(0.0) * PI * 1.6).cos();
    let mut fixing = act.pose.clone();
    let temple = m.eye + sx(side, m.eye_spacing * 0.5 + 0.055 * u, (0.03 - 0.07 * stroke) * u, -0.02 * u - 0.03 * u * stroke);
    hand_to_face(&mut fixing, ctx, side, temple, sx(side, -0.3, 0.9, -0.3), sx(side, 0.9, -0.2, 0.2));
    fixing.fingers(side, HandShape::FLAT.tighten(0.15));
    fixing.head(0.04, -0.1, 0.14);
    act.pose.blend(&fixing, reach);
    act.gaze = Vec2::new(-0.3 * reach, -0.2 * reach);
    act.attention = 1.0 - 0.7 * reach;
    act.face = Face { relaxed: 0.35 * reach, eyes_closed: 0.25 * reach, ..default() };
    act.walk_arms = 0.4;
    act
}

/// Pleased clapping, quick and bouncy, then hands held together.
pub fn clap(ctx: &Ctx) -> Act {
    let mut act = neutral(ctx);
    let (e, u) = (ctx.e, ctx.u());
    let clapping = 1.0 - smoothstep(1.8, 2.3, e);
    // About four claps a second.
    let open = 0.5 + 0.5 * (e * TAU * 4.0).cos();
    let apart = 0.02 + 0.14 * open * clapping;
    hands_together(&mut act.pose, ctx, 0.02, 0.19, apart, HandShape::FLAT);
    act.pose.spine(-0.03, 0.0, 0.0);
    act.pose.head(-0.05 + 0.03 * open * clapping, 0.0, 0.08);
    for side in Side::BOTH {
        act.feet[side.index()].heel = 0.3 * clapping * (1.0 - open);
    }
    act.hop = 0.03 * u * bump(e / 0.4);
    act.face = Face { happy: 1.0, aa: 0.2 * clapping, ..default() };
    act.walk_arms = 0.5;
    act
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emotes::Style;
    use crate::rig::Rig;

    #[test]
    fn moments_are_finite_from_start_to_settle() {
        let rig = Rig::synthetic(&[]);
        type Moment = fn(&Ctx) -> Act;
        let moments: [(&str, Moment); 6] =
            [("stretch", stretch), ("look_around", look_around), ("hum", hum), ("twirl", twirl), ("fix_hair", fix_hair), ("clap", clap)];
        for (name, moment) in moments {
            for step in 0..80 {
                let e = step as f32 * 0.1;
                let ctx = Ctx { rig: &rig, t: e + 3.0, e, style: Style::default(), talk: 0.0, energy: 0.0, beat: e * 2.0 };
                let act = moment(&ctx);
                assert!(act.pose.rot.iter().all(|q| q.is_finite() && q.is_normalized()), "{name} at {e}s");
                assert!(act.pose.hips.is_finite() && act.hop >= 0.0, "{name} at {e}s");
                assert!(act.spin.abs() < TAU, "{name}: a finished turn must read as 0");
            }
        }
    }

    #[test]
    fn the_stretch_goes_up_and_comes_back_down() {
        let rig = Rig::synthetic(&[]);
        let wrist_height = |e: f32| {
            let ctx = Ctx { rig: &rig, t: e, e, style: Style::default(), talk: 0.0, energy: 0.0, beat: 0.0 };
            Landmarks::of(&rig, &stretch(&ctx).pose).pos(crate::rig::Bone::LeftHand).y
        };
        let head = rig.rest(crate::rig::Bone::Head).y;
        assert!(wrist_height(1.5) > head, "hands above the head mid-stretch");
        assert!(wrist_height(6.0) < head - 0.3, "and back down after");
    }
}
