//! Regex inventories copied from pathway-materialization.ts for Rust promotion safety.
//! Keep behavior aligned until that source-of-truth logic is fully migrated.

use super::requirement_html::decode_transfer_planner_html_entities;
use super::requirement_shape::js_regex_compatibility_pattern;
use regex::Regex;
use std::sync::OnceLock;

const STRUCTURAL_ID_PATTERNS: &[&str] = &[
    r#"^(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^students?-.*-(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^classes?-in-this-.*-(?:option|track|route|pathway|certificate|concentration)-.*-offered\b"#,
    r#"^you-.*-(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^they-.*-(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^.+-can-also-be-taken-as-(?:a|an)-(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^.+-learn-more-about-.*-(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^and-.*-(?:option|track|route|pathway|certificate|concentration)(?:-|$)"#,
    r#"^if-you-enrolled-.*-option\b"#,
    r#"^the-.*-(?:option|track|route|pathway|certificate|concentration)-.*-(?:is|prepares)\b"#,
    r#"^the-.*-(?:option|track|route|pathway|certificate|concentration)-.*-(?:will-)?(?:thoroughly-)?prepare\b"#,
    r#"^and-.*-(?:option|track|route|pathway|certificate|concentration)-.*-(?:is|prepares)\b"#,
    r#"^extent-and-quality\b"#,
    r#"^credential-overview-(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^recommended-preparation-(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^with-honors-.*departmental-honors.*-(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^especially-those-who-.*-pathway$"#,
    r#"^\d+-of-the-\d+-credits-required-for-the-concentration$"#,
    r#"^\d+-second-.*-language-.*-concentration$"#,
    r#"^courses-by-track$"#,
    r#"(?:^|[-:])choose-from-the-following(?:$|[-:])"#,
    r#"^doctor(?:-|$)"#,
    r#"^ph-?d(?:-|$)"#,
    r#"^p-h-d(?:-|$)"#,
    r#"(?:^|[-:])not-admitting(?:$|[-:])"#,
    r#"^a-minor-.*explore-the-liberal-arts$"#,
    r#"^department-must-approve.*option$"#,
    r#"^m-ed\b"#,
    r#"^teaching-track$"#,
    r#"^with-the-option-to\b"#,
    r#"^and-.*-(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^although-.*-concentration-areas-.*-informal$"#,
    r#"^b-s-with-.*-option$"#,
    r#"^bs-option-family:"#,
    r#"^\d+-.*choose-one-option\b"#,
    r#"^option-\d+\b"#,
    r#"^page-\d+\b"#,
    r#"(?:^|[-:])option-specific(?:$|[-:])"#,
    r#"(?:^|[-:])track-specific(?:$|[-:])"#,
    r#"(?:^|[-:])pathway-specific(?:$|[-:])"#,
    r#"(?:^|[-:])choose-one-option(?:$|[-:])"#,
    r#"(?:^|[-:])choose-your(?:$|[-:])"#,
    r#"(?:^|[-:])choose-one(?:$|[-:])"#,
    r#"(?:^|[-:])declaring-an-option(?:$|[-:])"#,
    r#"(?:^|[-:])declaring-the(?:$|[-:])"#,
    r#"(?:^|[-:])how-to-declare(?:$|[-:])"#,
    r#"(?:^|[-:])how-do-i(?:$|[-:])"#,
    r#"^explore-.*-concentration-areas?"#,
    r#"^planning-.*-degree-electives?.*-concentration-area"#,
    r#"^what-is-the-difference-.*-concentration-area"#,
    r#"(?:^|[-:])declaration-process(?:$|[-:])"#,
    r#"(?:^|[-:])concentration-electives?(?:$|[-:])"#,
    r#"(?:^|[-:])core-courses?(?:$|[-:])"#,
    r#"(?:^|[-:])program-requirements?(?:$|[-:])"#,
    r#"(?:^|[-:])requirements-to-declare(?:$|[-:])"#,
    r#"(?:^|[-:])see-additional(?:$|[-:])"#,
    r#"(?:^|[-:])depending-on(?:$|[-:])"#,
    r#"(?:^|[-:])varies-by(?:$|[-:])"#,
    r#"(?:^|[-:])minimum-\d"#,
    r#"^may-benefit-from-completing-.*-track$"#,
    r#"^\d{1,3}(?:-\d{1,3})?-credits"#,
    r#"(?:^|[-:])option-.*electives?(?:$|[-:])"#,
    r#"^see-.*-(?:option|track|concentration|pathway)$"#,
    r#"^choose-thesis-project-or-course-only-option"#,
    r#"^transfer-students-apply-for-admission-under-this-pathway"#,
    r#"^funding-grad-school-track$"#,
    r#"^gis-resources-at-uw-track$"#,
    r#"^graduation-track$"#,
    r#"^home-track$"#,
    r#"^including-.*-option$"#,
    r#"^other-approved-.*electives?-option$"#,
    r#"^other-.*electives?-option$"#,
    r#"^additional-.*electives?-option$"#,
    r#"^topics-option$"#,
    r#"^.+-\d+(?:-\d+)?-option$"#,
    r#"^.+-no-longer-accepting-new-students\b"#,
    r#"^senior-electives-pathway$"#,
    r#"^special-projects?.*(?:graded-option-only|count-toward-the-option)"#,
    r#"^to-be-considered-.*-pathway$"#,
    r#"^who-should-choose-.*-option$"#,
    r#"^route-menu-active-trails(?:$|[-:])"#,
    r#"^route-name-is-layout-builder-ui(?:$|[-:])"#,
];

const GENERIC_LABEL_PATTERNS: &[&str] = &[
    r#"^(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^students?\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^classes?\s+in\s+this\b.*\b(?:option|track|route|pathway|certificate|concentration)\b.*\boffered\b"#,
    r#"^the\b.*\b(?:option|track|route|pathway|certificate|concentration)\b.*\b(?:provides|emphasizes|requires)\b"#,
    r#"^the\b.*\b(?:option|track|route|pathway|certificate|concentration)\b.*\b(?:is|prepares)\b"#,
    r#"^the\b.*\b(?:option|track|route|pathway|certificate|concentration)\b.*\b(?:will\s+)?(?:thoroughly\s+)?prepare\b"#,
    r#"^if you enrolled\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^if\s+adding\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^and\b.*\b(?:option|track|route|pathway|certificate|concentration)\b.*\b(?:is|prepares)\b"#,
    r#"^\d+\s+of\s+the\s+\d+\s+credits?\s+required\s+for\s+the\s+(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^\(?\d+\)?\s+second\b.*\blanguage\b.*\bconcentration\b"#,
    r#"^courses by track$"#,
    r#"\bchoose from the following\b"#,
    r#"^doctor(?:\s+of)?\b"#,
    r#"^ph\.?\s*d\.?\b"#,
    r#"^p\.?\s*h\.?\s*d\.?\b"#,
    r#"\bnot admitting\b"#,
    r#"^a minor\b.*\bexplore the liberal arts\b"#,
    r#"^department must approve\b.*\boption\b"#,
    r#"^m\.?\s*ed\.?\b"#,
    r#"^teaching track$"#,
    r#"^with the option to\b"#,
    r#"^and\b.*\b(?:option|track|route|pathway|certificate|concentration)$"#,
    r#"^according to the option chosen\b"#,
    r#"^additional (?:admission|completion )?requirements?\b"#,
    r#"^advanced data science option specific requirements?\b"#,
    r#"^admission under\b"#,
    r#"^after completing\b.*\boption\b"#,
    r#"^all pathways\b"#,
    r#"^although\b.*\bconcentration areas?\b.*\binformal\b"#,
    r#"^an option$"#,
    r#"^and the concentration coordinator\b"#,
    r#"^a general description of\b.*\bconcentration\b"#,
    r#"^begin taking\b.*\boption classes\b"#,
    r#"^b\.?\s*s\.?\s+with\b.*\boption\b"#,
    r#"^budget analysts\b.*\boption\b"#,
    r#"^complete the requirements\b"#,
    r#"^clinical requirements?\b"#,
    r#"^concentration course numbers?\b"#,
    r#"^concentration electives?\b"#,
    r#"^concentration area courses?\b"#,
    r#"^concentration projects?\b"#,
    r#"^concentration\s+[ivxlcdm]+\b(?:\s+courses?)?(?:\s*[:.]?)?$"#,
    r#"^(?:optional\s+)?(?:concentration|focus)\s+areas?$"#,
    r#"^core courses?\b"#,
    r#"^course lists?\b"#,
    r#"^course[- ]only option\b"#,
    r#"^create your own pathway as a separate option\b"#,
    r#"^.{2,80}\bcurriculum option\b"#,
    r#"^curriculum and instruction\):"#,
    r#"^(?:examples of\s+)?coursework pathways?\b"#,
    r#"^degree options?\b"#,
    r#"^electives?\b.*\bchoose\b"#,
    r#"^capstone experience\b"#,
    r#"^capstone option\b"#,
    r#"^choose (?:one|two|three|between|from)\b"#,
    r#"^choose your\b"#,
    r#"^contact the\b"#,
    r#"^degree options?\b"#,
    r#"^declaring an option\b"#,
    r#"^declaring(?: the)?\b"#,
    r#"^declare your major option\b"#,
    r#"^download\b"#,
    r#"^explore\b.*\bconcentration areas?\b"#,
    r#"^electives for\b.*\boption\b"#,
    r#"^formal options?\b"#,
    r#"^followed by\b.*\boption\b"#,
    r#"^funding grad school\b"#,
    r#"\bfee[- ]based\b"#,
    r#"^gis resources at uw\b"#,
    r#"^graduation\b"#,
    r#"^home\b"#,
    r#"^how do i\b.*\bconcentration area\b"#,
    r#"^how to declare(?: the)?\b"#,
    r#"^including\b.*\boption\b"#,
    r#"^joining the\b"#,
    r#"^me option courses?\b"#,
    r#"^master(?:\s+of)?\b"#,
    r#"^marketing management to declare\b.*\bconcentration\b"#,
    r#"^.+\bcan also be taken as (?:an? )?(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^.+\blearn more about\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^may benefit from completing\b.*\btrack\b"#,
    r#"^minimum \d"#,
    r#"^listed below are .*coursework pathways?\b"#,
    r#"^non-me courses as me option\b"#,
    r#"^note:?\s+prerequisites? for option\b"#,
    r#"^\d{1,3}(?:-\d{1,3})?\s*credits?\b"#,
    r#"^\d{1,3}(?:-\d{1,3})?\s+credits?\s+depending on option\b"#,
    r#"^option and concentration curriculum\b"#,
    r#"^.{2,80}\boption:\s+.*electives?\b"#,
    r#"^option course numbers?\b"#,
    r#"^option courses?\b"#,
    r#"^option \d+\b"#,
    r#"^option-specific\b"#,
    r#"^or from\b.*\bconcentration\b"#,
    r#"^or course[- ]only option\b"#,
    r#"^other approved\b.*\belectives?\s+option\b"#,
    r#"^other\b.*\belectives?\s+option\b"#,
    r#"^additional\b.*\belectives?\s+option\b"#,
    r#"^page \d+\b"#,
    r#"^\(?\d+\)?\s*plan a pathway toward\b"#,
    r#"^planning\b.*\bdegree electives?\b.*\bconcentration area\b"#,
    r#"^please see\b.*\b(?:option page|courses? by track)\b"#,
    r#"^please check out\b"#,
    r#"^possible coursework pathways?\b"#,
    r#"^option[- ]specific\b"#,
    r#"^pathway[- ]specific\b"#,
    r#"^ph concentration projects?\b"#,
    r#"^requirements? to declare\b"#,
    r#"^required\b.*\boption courses?\b"#,
    r#"^see ["']?additional\b"#,
    r#"^see\b.*\b(?:option|track|concentration|pathway)\b"#,
    r#"^selection of\b.*\boption\b"#,
    r#"^specific areas covered within\b.*\boption\b"#,
    r#"^supplemental official\b"#,
    r#"^students?.*\bmust complete\b.*\bfor graduation\b"#,
    r#"^this track emphasizes\b"#,
    r#"^this option (?:also )?requires\b"#,
    r#"^this option explores\b"#,
    r#"^the courses in\b.*\boption\b.*\bcover topics such as\b"#,
    r#"^the rationale for\b.*\bconcentration\b"#,
    r#"^teaching track faculty\b"#,
    r#"^tenure track\b"#,
    r#"^to declare\b.*\b(?:option|concentration)\b"#,
    r#"^to propose\b.*\bconcentration\b"#,
    r#"^track[- ]specific\b"#,
    r#"^track specific\b"#,
    r#"^track-specific\b"#,
    r#"^upon completing\b.*\boption\b"#,
    r#"^you have the option to\b"#,
    r#"^gis certificate\b.*complete all\b"#,
    r#"^foundation courses?\b"#,
    r#"^gis certificate classes\b"#,
    r#"^in addition to\b"#,
    r#"^option breadth\b"#,
    r#"^option[- ]specific\b"#,
    r#"^program requirements?\b"#,
    r#"^program option$"#,
    r#"^plus\b"#,
    r#"^students declare\b"#,
    r#"^the curriculum consists\b"#,
    r#"^why choose\b"#,
    r#"^what is the difference\b.*\bconcentration area\b"#,
    r#"^who should choose\b"#,
    r#"^which is detailed at\b"#,
    r#"^senior electives?\b"#,
    r#"^topics option$"#,
    r#"^special projects?\b.*\b(?:graded option only|count toward the option)\b"#,
    r#"^to be considered\b.*\bpathway\b"#,
    r#"\bdouble major\b"#,
    r#"\bdouble degree\b"#,
    r#"\b(?:clinical|didactic) coursework\b"#,
    r#"\boption[- ]specific\b.*\b(?:credits?|requirements?|coursework)\b"#,
    r#"\b(?:option|track|concentration) course numbers?\b"#,
    r#"\b(?:option|track|concentration) courses?\b"#,
    r#"\bno longer accepting new students\b"#,
    r#"\bconcentration projects?$"#,
    r#"\bstudents? must complete\b.*\bfor graduation\b"#,
    r#"\btrack[- ]specific\b.*\b(?:credits?|requirements?|coursework)\b"#,
    r#"\btrack\s*\(addl\.\s*\d"#,
    r#"\btrack electives?\b"#,
    r#"\bdepending on (?:credential/)?option\b"#,
    r#"\bvaries by option\b"#,
    r#"\bchoose thesis, project, or course-only option\b"#,
    r#"^for admission requirements\b.*\b(?:concentration|option|track|route|pathway)\b"#,
    r#"^for the\b.*\b(?:concentration|option|track|route|pathway)\b"#,
    r#"^(?:[†*§◊]+)?\s*if not taken\b.*\b(?:concentration|option|track|route|pathway)\b"#,
    r#"^concentration\s+[ivxlcdm]+\b.*\b(?:credits?|courses?)\b"#,
    r#"^(?:[â€ *Â§â—Š]+\s*)?route\.menu_active_trails:"#,
    r#"^(?:[â€ *Â§â—Š]+\s*)?route\.name\.is_layout_builder_ui\b"#,
    r#"^route\s+name\s+is\s+layout\s+builder\s+ui\b"#,
    r#"\bmenu[_-]?active[_-]?trails\b"#,
    r#"\blayout[_-]?builder[_-]?ui\b"#,
    r#"^you\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^they\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^and\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^extent and quality\b"#,
    r#"^credential overview\b"#,
    r#"^recommended preparation\b"#,
    r#"^with honors\b.*\bdepartmental honors\b.*\b(?:option|track|route|pathway|certificate|concentration)\b"#,
    r#"^especially those who\b.*\bpathway\b"#,
    r#"\bcompletion of departmental honors requirements in the major\b"#,
];

fn compile_patterns(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|pattern| {
            Regex::new(&js_regex_compatibility_pattern(&format!("(?i){pattern}"))).unwrap_or_else(
                |error| panic!("invalid structural pathway pattern {pattern:?}: {error}"),
            )
        })
        .collect()
}

fn normalize(value: &str) -> String {
    decode_transfer_planner_html_entities(value)
}

pub fn is_suspicious_structural_pathway_id(value: &str) -> bool {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    static BA_BS_EXCEPTION: OnceLock<Regex> = OnceLock::new();
    let normalized = normalize(value).to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    if BA_BS_EXCEPTION
        .get_or_init(|| {
            Regex::new(r"(?i)^(?:ba|bs)-option-family:[a-z0-9][a-z0-9-]*$")
                .expect("valid BA/BS exception")
        })
        .is_match(&normalized)
    {
        return false;
    }
    PATTERNS
        .get_or_init(|| compile_patterns(STRUCTURAL_ID_PATTERNS))
        .iter()
        .any(|pattern| pattern.is_match(&normalized))
}

pub fn is_suspicious_structural_pathway_label(value: &str) -> bool {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    static PDF_PATTERN: OnceLock<Regex> = OnceLock::new();
    static GENERIC_REQUIREMENTS_PATTERN: OnceLock<Regex> = OnceLock::new();
    let normalized = normalize(value);
    if normalized.is_empty() {
        return false;
    }
    if PDF_PATTERN.get_or_init(|| Regex::new(r"(?i)(?-u:\b)(?:pdf|worksheet|check\s*list|checklist)(?-u:\b)").expect("valid document-label pattern")).is_match(&normalized)
        || normalized.starts_with('>')
        || has_unmatched_trailing_parenthesis(&normalized)
        || GENERIC_REQUIREMENTS_PATTERN.get_or_init(|| Regex::new(r"(?i)(?-u:\b)(?:option|track|route|pathway|certificate|concentration)(?-u:\b).*?(?-u:\b)requirements?(?-u:\b)").expect("valid requirement-label pattern")).is_match(&normalized)
    {
        return true;
    }
    PATTERNS
        .get_or_init(|| compile_patterns(GENERIC_LABEL_PATTERNS))
        .iter()
        .any(|pattern| pattern.is_match(&normalized))
}

fn has_unmatched_trailing_parenthesis(value: &str) -> bool {
    value.ends_with(')')
        && value.chars().filter(|character| *character == ')').count()
            > value.chars().filter(|character| *character == '(').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_source_materialization_patterns_compile() {
        assert_eq!(compile_patterns(STRUCTURAL_ID_PATTERNS).len(), 82);
        assert_eq!(compile_patterns(GENERIC_LABEL_PATTERNS).len(), 182);
    }

    #[test]
    fn source_materialization_safety_examples_are_preserved() {
        assert!(is_suspicious_structural_pathway_id(
            "2-of-the-4-credits-required-for-the-concentration"
        ));
        assert!(!is_suspicious_structural_pathway_id(
            "bs-option-family:general-biology"
        ));
        assert!(is_suspicious_structural_pathway_label(
            "Double major and degree requirements"
        ));
        assert!(is_suspicious_structural_pathway_label(
            "Download the pathway PDF"
        ));
        assert!(is_suspicious_structural_pathway_label(
            "Double major &amp; degree requirements"
        ));
        assert!(is_suspicious_structural_pathway_label(
            "Requirements for the Biology Option (track))"
        ));
        assert!(is_suspicious_structural_pathway_label(
            "Course option requirements"
        ));
        assert!(!is_suspicious_structural_pathway_label(
            "Degree requirements"
        ));
        assert!(!is_suspicious_structural_pathway_label(
            "Data Science Option"
        ));
    }
}
