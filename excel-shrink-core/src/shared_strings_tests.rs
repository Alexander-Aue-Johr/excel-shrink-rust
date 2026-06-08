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