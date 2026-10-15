use super::*;

#[test]
fn manual_choices_cover_supported_outputs_and_can_find_keys_missing_from_a_keyboard() {
    for locale in ["en", "zh-CN"] {
        let choices = output_options(locale, "");
        let mut values = std::collections::BTreeSet::new();
        for choice in choices {
            assert!(values.insert(choice.value.to_string()));
            let output = output_from_choice(&choice.value).unwrap();
            assert_eq!(output_option(locale, output), choice);
        }
        let f24 = output_options(locale, " f24 ");
        assert_eq!(f24.len(), 1);
        assert_eq!(
            output_from_choice(&f24[0].value),
            Some(MappingOutput::Keyboard {
                usage: 0x73,
                modifiers: 0
            })
        );
        assert!(output_options(locale, "nonexistent key").is_empty());
    }
    let side2 = output_options("zh-CN", "侧键 2");
    assert_eq!(side2.len(), 1);
    assert_eq!(
        output_from_choice(&side2[0].value),
        Some(MappingOutput::Mouse {
            button: MouseButton::Side2
        })
    );
    for invalid in [
        "key/0", "key/255", "key/no", "mouse/5", "mouse/-1", "other/1",
    ] {
        assert_eq!(output_from_choice(invalid), None);
    }
}
