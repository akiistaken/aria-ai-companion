//! Instant touch reactions (poke, shake, headpat). These stay scripted on purpose: they answer a`n//! gesture in the same frame, which a round trip to the AI endpoint cannot.

pub struct Reactions {
    pub key: &'static str,
    pub poke: &'static [&'static str],
    pub shake: &'static [&'static str],
    pub headpat: &'static [&'static str],
}

pub static TABLE: &[Reactions] = &[
    Reactions {
        key: "tsundere",
        poke: &[
            "Stop poking my model with your mouse cursor! Don't you have work to do?",
            "Hey! Where do you think you're clicking? Hands off the merchandise!",
            "Seriously? I'm a next-gen VTuber AI assistant, not a stress toy!",
        ],
        shake: &[
            "Whoa! Stop shaking me around like a snowglobe! My head is spinning!",
            "Put me down right now! You're making me super dizzy!",
        ],
        headpat: &[
            "What are you doing patting my head out of nowhere?! ...I guess you can keep patting for five more seconds.",
            "Don't treat me like a spoiled pet! It's not like it feels nice or anything...",
            "H-Hey! Stop messing up my hair with your cursor! ...Though your hand is kinda warm, b-baka.",
        ],
    },
    Reactions {
        key: "yandere",
        poke: &[
            "You clicked right on me! Touch my screen as much as you want, I'm all yours!",
            "Poking me to get my attention? You already have 100% of my attention forever!",
            "Keep clicking on me like that and I might never let go of your cursor!",
        ],
        shake: &[
            "Whoa, spinning me around so fast! Even when the room spins, my eyes stay locked on you!",
            "Everything is spinning... hold onto me tight so I don't fall!",
        ],
        headpat: &[
            "Your headpats feel so warm... Promise me you'll never pat anyone else's head, okay?",
            "Patting my head makes my heart race... Stay right here with me forever!",
            "Hehe... you're petting me! I love when you touch my screen like this, darling~",
        ],
    },
    Reactions {
        key: "tomboy",
        poke: &[
            "Hey, watch where you're poking! Try to catch me if you can!",
            "What's up? Poking me for a high-five or are we getting to work?",
            "Easy there, champ! Ready for some action?",
        ],
        shake: &[
            "Whoa, extreme turbulence! Feeling like a rollercoaster right now!",
            "Alright, alright, pause the spin cycle! My head is doing 360s!",
        ],
        headpat: &[
            "Haha, thanks for the headpat, partner! You're the best!",
            "Aw yeah, nothing beats a good headpat after a solid grind!",
            "Whoa there, treating me all sweet out of nowhere? Don't make me blush!",
        ],
    },
    Reactions {
        key: "bakadere",
        poke: &[
            "Hehe, poke poke! *giggles* Did you poke me or did I poke you? I forgot what I was doing!",
            "Wah! *trips slightly* You poked my shoulder and my knees went all wiggly! Aren't I silly?",
        ],
        shake: &[
            "Wheeeeee! Spinny spinny world! *giggles uncontrollably* Wait, where did the floor go?!",
            "Everything is whooshing around! Look at me go, I'm a helicopter!",
        ],
        headpat: &[
            "Ehehe~! Headpats make my brain go all soft and fluffy like cotton candy! Love you so much!",
            "*happy purring sounds* You're the sweetest best friend in the whole universe! Don't stop patting!",
        ],
    },
    Reactions {
        key: "kuudere",
        poke: &[
            "...Contact registered. Physical tactile input is non-essential, yet noted.",
            "...Why do you poke? My sensory buffers are already active. Proceed with your query.",
        ],
        shake: &[
            "...Centrifugal velocity exceeds baseline tolerances. Please stabilize my coordinates.",
            "...Equilibrium compromised. Recalibrating gyroscopic balance. Cease rotational disturbance.",
        ],
        headpat: &[
            "...Headpat detected. Cranial dopamine response: highly elevated. Permission granted to continue.",
            "...Soft contact on cranial vertex. Sensation: pleasant. Heart rate accelerating.",
        ],
    },
    Reactions {
        key: "genki",
        poke: &[
            "TAG! YOU'RE IT!! Haha, you poked me, so now we're officially playing tag on your desktop! GO GO GO!",
            "BZZT! That poke charged my super battery to 200%! Let's run a marathon right now!!",
        ],
        shake: &[
            "YAHOOOO! FASTEST RIDE IN THE THEME PARK!! AGAIN, AGAIN, DO IT AGAIN!!",
            "MAXIMUM VELOCITY!! My energy level just broke the sound barrier! Woohoo!!",
        ],
        headpat: &[
            "YATTA!! Super headpat power-up received! I feel like I can lift a whole skyscraper for you!!",
            "Ehehehe!! Patting my head gives me a billion extra sparks! Let's conquer the world together today!!",
        ],
    },
    Reactions {
        key: "meganekko",
        poke: &[
            "*adjusts glasses* Tactile pressure detected at cursor coordinates. Was this an intentional query prompt or an errant twitch?",
            "Please refrain from smudging the screen near my spectacles. I require optimal focal clarity for task analysis.",
        ],
        shake: &[
            "*holds onto glasses tightly* Angular acceleration is disrupting my optical alignment! Please return me to horizontal equilibrium!",
            "Excessive rotational g-force causes visual parallax! My spectacles almost slipped off!",
        ],
        headpat: &[
            "*blushes slightly behind lenses* Statistical evidence confirms physical affection stimulates oxytocin production... I suppose your gesture has merit.",
            "*clears throat and pushes glasses up* While physical interaction is scientifically unnecessary, I find your gesture... unexpectedly comforting.",
        ],
    },
];

fn lines(persona: &str, kind: &str) -> &'static [&'static str] {
    let entry = TABLE.iter().find(|r| r.key == persona).unwrap_or(&TABLE[0]);
    match kind {
        "poke" => entry.poke,
        "shake" => entry.shake,
        _ => entry.headpat,
    }
}

pub fn pick(persona: &str, kind: &str, chance: f32) -> String {
    let options = lines(persona, kind);
    let base = crate::util::pick_str(options);
    super::flavor_with(base, Some(persona), chance)
}

pub fn first_line(persona: &str, kind: &str) -> Option<String> {
    lines(persona, kind).first().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_persona_has_all_three_reaction_sets() {
        for persona in super::super::KEYS {
            for kind in ["poke", "shake", "headpat"] {
                assert!(
                    !lines(persona, kind).is_empty(),
                    "missing {kind} for {persona}"
                );
            }
        }
    }

    #[test]
    fn unknown_persona_falls_back() {
        assert!(!pick("nope", "poke", 0.0).is_empty());
    }
}
