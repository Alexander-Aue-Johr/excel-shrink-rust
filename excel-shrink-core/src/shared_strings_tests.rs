use super::*;

#[test]
fn detects_only_whitespace_shared_strings() {
    let xml = br#"
        <sst>
            <si><t>   </t></si>
            <si><t>hello</t></si>
            <si><t>
            </t></si>
            <si><t>	</t></si>
        </sst>
    "#;

    let indices = find_whitespace_sst_indices(xml);

    assert_eq!(indices, vec![0, 2, 3]);
}

#[test]
fn detects_self_closing_empty_text_shared_string_as_whitespace() {
    let xml = br#"
        <sst>
            <si><t /></si>
            <si><t>hello</t></si>
        </sst>
    "#;

    let indices = find_whitespace_sst_indices(xml);

    assert_eq!(indices, vec![0]);
}

#[test]
fn detects_rich_text_shared_string_as_whitespace_only_when_all_runs_are_whitespace() {
    let xml = br#"
        <sst>
            <si><r><t> </t></r><r><t>&#10;</t></r></si>
            <si><r><t> </t></r><r><t>x</t></r></si>
            <si><r><t>	</t></r><r><t>
</t></r></si>
        </sst>
    "#;

    let indices = find_whitespace_sst_indices(xml);

    assert_eq!(indices, vec![2]);
}

#[test]
fn ignores_similarly_named_tags_when_finding_shared_strings() {
    let xml = br#"
        <sst>
            <single><t> </t></single>
            <siExtra><t> </t></siExtra>
            <si><text>hello</text></si>
            <si><t> </t></si>
        </sst>
    "#;

    let indices = find_whitespace_sst_indices(xml);

    assert_eq!(indices, vec![0, 1]);
}
