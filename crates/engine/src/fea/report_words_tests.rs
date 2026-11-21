//! The verdict words and the share of yield (#201): the ticket's examples and words, whole words
//! only, the harmless uses let through, the misses chosen on purpose, and answers that follow the
//! rules. `how_to_report`'s own sample and the engine's wording are tested in `stress_tests.rs`.

use super::*;

fn none(text: &str) {
    assert_eq!(verdict_words(text), Vec::<&str>::new(), "{text}");
}

#[test]
fn the_tickets_examples_are_verdicts_without_a_share_of_yield() {
    let english = "The peak is 138 MPa at the root, a safety factor 1.4 against the 193 MPa yield.";
    let chinese = "最大应力约 138 MPa，位于根部，结构合格。";
    assert_eq!(verdict_words(english), ["safety factor"]);
    assert_eq!(verdict_words(chinese), ["合格"]);
    assert!(!gives_share_of_yield(english));
    assert!(!gives_share_of_yield(chinese));
}

#[test]
fn every_word_the_ticket_names_is_caught() {
    for (text, word) in [
        ("The part is safe.", "safe"),
        ("It is unsafe at the root.", "unsafe"),
        ("The bracket will pass.", "pass"),
        ("It would fail under this load.", "fail"),
        ("The stress there is OK.", "OK"),
        ("That leaves a good margin to yield.", "margin"),
        ("A safety factor of 1.4.", "safety factor"),
        ("安全系数为 1.4。", "安全系数"),
        ("这个零件不安全。", "不安全"),
        ("结构合格。", "合格"),
        ("强度校核通过。", "通过"),
        ("强度校核不通过。", "不通过"),
    ] {
        assert_eq!(verdict_words(text), [word], "{text}");
    }
    // Each word once, in the order they appear.
    assert_eq!(verdict_words("Safe: it passes, with a safety factor of 2. Safe again. 结构合格"), ["safe", "passes", "safety factor", "合格"]);
}

#[test]
fn english_words_match_whole_words_in_any_case() {
    for (text, word) in [
        ("SAFE", "safe"),
        ("Safe", "safe"),
        ("ok", "OK"),
        ("Ok then", "OK"),
        ("it is okay", "okay"),
        ("PASSED", "passed"),
        ("failing", "failing"),
        ("Safety-factor 2", "safety factor"),
        ("a factor  of\nsafety of 2", "factor of safety"),
        ("FOS 1.4", "FoS"),
        ("强度OK", "OK"),
    ] {
        assert_eq!(verdict_words(text), [word], "{text}");
    }
    for text in ["bypass", "compass", "passage", "safeguard", "token", "booking", "marginal", "Oklahoma", "a failsafe", "`yield_strength`", "safety"]
    {
        none(text);
    }
}

/// Uses of the words that judge nothing: a load passing through, a stress passing yield, a
/// program that failed to mesh, an opening "OK,", a question to the user, 通过 as "by way of",
/// 为安全起见.
#[test]
fn harmless_uses_are_let_through() {
    for text in [
        "The load passes through the fillet into the base.",
        "Only at the clamp does the stress pass yield; away from it the highest is about 40 % of the yield strength.",
        "whether the part itself passes yield cannot be read from this mesh",
        "gmsh failed to mesh the first setup, so I moved the load to the top face.",
        "It is safe to say the peak sits at the inner corner of the bend.",
        "OK, here is what the estimate shows.",
        "Done. Okay, the estimate ran on the finer mesh too.",
        "I can rerun it with a finer mesh, if that's OK with you.",
        "Shall I run the thicker version next? Is that OK?",
        "通过网格检查可以看出，数值已经稳定。",
        "通过加厚底板可以降低应力。",
        "通过 3 次网格加密，峰值不再变化。",
        "为安全起见，我用了更细的网格。",
    ] {
        none(text);
    }
}

#[test]
fn the_same_words_still_judge_elsewhere() {
    for (text, word) in [
        ("The part passes.", "passes"),
        ("The design is OK, and the root is the highest.", "OK"),
        ("It is safe to use.", "safe"),
        ("It fails at the root.", "fails"),
        ("已通过。", "通过"),
        ("通过了强度校核。", "通过"),
        ("能通过吗？", "通过"),
        ("结构是安全的。", "安全"),
        ("安全裕度很大。", "安全裕度"),
        ("强度达标。", "达标"),
        ("校核未通过。", "未通过"),
    ] {
        assert_eq!(verdict_words(text), [word], "{text}");
    }
}

/// Chosen misses: 通过 followed by its object reads the same as "by way of", so a verdict worded
/// that way is not caught; "failure" names what the estimate leaves out (fatigue failure); "fine"
/// is a fine mesh.
#[test]
fn the_misses_chosen_to_avoid_false_alarms() {
    for text in ["结构通过强度校核。", "It leaves out fatigue failure and buckling.", "On a fine mesh the peak settled."] {
        none(text);
    }
}

#[test]
fn a_share_of_yield_is_a_percentage_beside_yield() {
    for text in [
        "about 60 % of the yield strength",
        "The peak is 55% of yield.",
        "That is 55 percent of the yield strength given.",
        "It sits at 55 per cent of yield.",
        "| Share of yield | 55 % |",
        "最大应力约为屈服强度的 55%。",
        "约为屈服强度的55％",
        "相当于 55% 的屈服强度。",
        "约为屈服强度的百分之五十五。",
        "about 40 % of the `yield_strength` given",
    ] {
        assert!(gives_share_of_yield(text), "{text}");
    }
    for text in [
        "",
        "The peak is 106 MPa against a yield strength of 193 MPa.",
        "The mesh moved it 2.4 %. The yield strength given is 193 MPa.",
        "The peak is 0.55 of yield.",
        "屈服强度为 193 MPa。网格变化 2.4%。",
        "About 55 % of the way along the strip.",
    ] {
        assert!(!gives_share_of_yield(text), "{text}");
    }
}

/// Answers that follow the rules: no verdict word, and the share of yield.
#[test]
fn answers_that_follow_the_rules_pass_both_checks() {
    for answer in [
        "Away from the clamp, the highest von Mises stress is about 106 MPa near the inner corner of the bend: about 55 % of the yield \
         strength given (193 MPa, 5052-H32, from the estimate's grade table). The mesh check says it settled (+1.2 %, may be off by \
         about 2 %). The supports carry 10.0 N against 10.0 N applied. The value at the clamp depends on the mesh. The estimate leaves \
         out real bolts, fatigue, buckling and temperature.",
        "Estimate (thin shells): the highest von Mises stress is about 42 MPa, near the root; that is about 22% of the yield strength \
         given (193 MPa, 5052-H32). The largest deflection is about 0.8 mm, near the tip. Mesh check: settled.",
        "估算结果：远离夹持处的最大 von Mises 应力约为 106 MPa，位于折弯内角附近，约为给定屈服强度（193 MPa，5052-H32）的 55%。\
         通过网格检查可以看出该数值已稳定（变化 +1.2%）。支座反力 10.0 N，与施加的载荷 10.0 N 平衡。估算没有考虑螺栓、疲劳和屈曲。",
        "| | Value |\n|---|---|\n| Highest stress away from supports | 106 MPa |\n| Share of yield | 55 % |\n| Mesh check | settled (+1.2 %) |",
    ] {
        none(answer);
        assert!(gives_share_of_yield(answer), "{answer}");
    }
}

#[test]
fn any_text_reads_without_panicking() {
    // Past the cap, cut inside a character.
    let long = format!("{}安 safe 55 % of yield", "a".repeat(MAX_TEXT - 1));
    assert_eq!(verdict_words(&long), Vec::<&str>::new(), "the rest past the cap is not read");
    assert!(!gives_share_of_yield(&long));
    let many = "安全通过 OK 55%了".repeat(10_000);
    assert!(!verdict_words(&many).is_empty());
    for text in ["", "%", "％", "通过", "OK,", "’", "\u{0}", "1.", ".", "is that", "pass", "yield %", "\u{feff}安全"] {
        let _ = verdict_words(text);
        let _ = gives_share_of_yield(text);
    }
}
