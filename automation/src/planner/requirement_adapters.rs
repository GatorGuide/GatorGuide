use super::requirement_docx::is_docx_source_url;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequirementSourceParserKind {
    Html,
    Docx,
    Pdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceAdapter {
    pub id: &'static str,
    pub family: &'static str,
    pub parser: RequirementSourceParserKind,
}

const UW_SEATTLE_HTML: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-seattle-html-degree-page",
    family: "UW Seattle HTML degree pages",
    parser: RequirementSourceParserKind::Html,
};
const UW_SEATTLE_CATALOG: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-seattle-catalog-page",
    family: "UW Seattle catalog pages",
    parser: RequirementSourceParserKind::Html,
};
const UW_BOTHELL_HTML: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-bothell-html-degree-page",
    family: "UW Bothell HTML degree pages",
    parser: RequirementSourceParserKind::Html,
};
const UW_BOTHELL_CATALOG: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-bothell-catalog-page",
    family: "UW Bothell catalog pages",
    parser: RequirementSourceParserKind::Html,
};
const GENERIC_DOCX: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "generic-official-docx-degree-sheet",
    family: "Generic official DOCX degree sheets",
    parser: RequirementSourceParserKind::Docx,
};
const UW_BOTHELL_PDF: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-bothell-pdf-worksheet",
    family: "UW Bothell PDF worksheets",
    parser: RequirementSourceParserKind::Pdf,
};
const UW_TACOMA_HTML: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-tacoma-html-degree-page",
    family: "UW Tacoma HTML degree pages",
    parser: RequirementSourceParserKind::Html,
};
const UW_TACOMA_CATALOG: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "uw-tacoma-catalog-page",
    family: "UW Tacoma catalog pages",
    parser: RequirementSourceParserKind::Html,
};
const GENERIC_PDF: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "generic-official-pdf-degree-sheet",
    family: "Generic official PDF degree sheets",
    parser: RequirementSourceParserKind::Pdf,
};
const GENERIC_HTML: RequirementSourceAdapter = RequirementSourceAdapter {
    id: "generic-official-html-page",
    family: "Generic official HTML pages",
    parser: RequirementSourceParserKind::Html,
};

pub fn select_requirement_source_adapter(entry: &Value) -> RequirementSourceAdapter {
    let campus_id = entry
        .get("campusId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let parser_type = entry
        .get("parserType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
    let html_degree_type = matches!(
        parser_type,
        "html-degree-page" | "html-curriculum-page" | "html-overview-page"
    );
    let pdf_type = matches!(
        parser_type,
        "pdf-degree-sheet" | "pdf-worksheet" | "generic-pdf"
    );

    if campus_id == "uw-seattle" && html_degree_type {
        UW_SEATTLE_HTML
    } else if campus_id == "uw-seattle" && parser_type == "catalog-page" {
        UW_SEATTLE_CATALOG
    } else if campus_id == "uw-bothell" && html_degree_type {
        UW_BOTHELL_HTML
    } else if campus_id == "uw-bothell" && parser_type == "catalog-page" {
        UW_BOTHELL_CATALOG
    } else if is_docx_source_url(url) {
        GENERIC_DOCX
    } else if campus_id == "uw-bothell" && pdf_type {
        UW_BOTHELL_PDF
    } else if campus_id == "uw-tacoma" && html_degree_type {
        UW_TACOMA_HTML
    } else if campus_id == "uw-tacoma" && parser_type == "catalog-page" {
        UW_TACOMA_CATALOG
    } else if pdf_type {
        GENERIC_PDF
    } else {
        GENERIC_HTML
    }
}
