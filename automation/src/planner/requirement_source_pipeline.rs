use super::requirement_adapters::{
    RequirementSourceAdapter, RequirementSourceParserKind, select_requirement_source_adapter,
};
use super::requirement_course_codes::CourseCodeParser;
use super::requirement_document_facts::{
    RequirementSourceTextFacts, collect_docx_source_facts, collect_html_source_facts,
    collect_pdf_source_facts,
};
use super::requirement_docx::{DocxSourceParser, is_pdf_source_url};
use super::requirement_html_source::HtmlSourceParser;
use super::requirement_pdf_layout::PdfPageBlock;
use super::requirement_manifest_context::RequirementSourceManifestContext;
use super::requirement_role_classifier::classify_requirement_source_role;
use super::requirement_recovery_links::extract_supplemental_html_link_candidates;
use super::requirement_source_fetching::SourceDownloader;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourcePipelineResult {
    pub adapter: RequirementSourceAdapter,
    pub source_role: &'static str,
    pub facts: RequirementSourceTextFacts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceRecoveryCandidates {
    pub generic_section_candidates: Vec<Value>,
    pub parser_recovery_links: Vec<Value>,
    pub supplemental_html_links: Vec<Value>,
    pub supplemental_document_links: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementManifestEntrySourceFacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_title: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<Value>,
    pub pathway_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campus_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_parser_type: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_source_url: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_source_label: Option<Value>,
    pub is_primary_degree_requirements_link: bool,
    pub structured_uw_course_codes: Vec<String>,
    #[serde(flatten)]
    pub parsed: RequirementSourcePipelineResult,
}

#[derive(Clone)]
pub struct RequirementSourcePipeline {
    html_parser: HtmlSourceParser,
    docx_parser: DocxSourceParser,
}

impl RequirementSourcePipeline {
    pub fn new(
        downloader: SourceDownloader,
        temporary_root: impl Into<PathBuf>,
        cache_capacity: usize,
    ) -> Self {
        Self {
            html_parser: HtmlSourceParser::with_cache_capacity(
                downloader.clone(),
                cache_capacity,
            ),
            docx_parser: DocxSourceParser::with_cache_capacity(
                downloader,
                temporary_root,
                cache_capacity,
            ),
        }
    }

    pub fn collect_manifest_entry_source_facts(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementManifestEntrySourceFacts, String> {
        let parsed = self.collect_source_facts(entry, timeout, pdf_page_blocks, course_parser)?;
        Ok(RequirementManifestEntrySourceFacts {
            owner_id: entry.get("ownerId").cloned(),
            owner_title: entry.get("ownerTitle").cloned(),
            plan_id: entry.get("planId").cloned(),
            pathway_id: context.materializable_pathway_id(entry),
            campus_id: entry.get("campusId").cloned(),
            primary_parser_type: entry.get("parserType").cloned(),
            primary_source_url: entry.get("url").cloned(),
            primary_source_label: entry.get("label").cloned(),
            is_primary_degree_requirements_link: js_truthy(entry.get("isPrimaryDegreeRequirementsLink")),
            structured_uw_course_codes: context.structured_uw_course_codes(entry, course_parser),
            parsed,
        })
    }

    pub fn collect_parser_recovery_section_lines(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<String>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.scope_parser_recovery_section_lines(
            entry,
            &artifacts.lines,
            course_parser,
        ))
    }

    pub fn collect_generic_parser_recovery_section_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.generic_parser_recovery_section_candidates(
            entry,
            &artifacts.lines,
            artifacts.title.as_deref(),
            &artifacts.headings,
            course_parser,
        ))
    }

    pub fn collect_parser_recovery_link_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.extract_parser_recovery_link_candidates(
            entry,
            &artifacts.links,
            course_parser,
        ))
    }

    pub fn collect_supplemental_html_link_candidates(
        &self,
        entry: &Value,
        timeout: Duration,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(extract_supplemental_html_link_candidates(entry, &artifacts.links))
    }

    pub fn collect_supplemental_document_link_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<Value>, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(Vec::new());
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(context.extract_supplemental_document_link_candidates(
            entry,
            &artifacts.links,
            course_parser,
        ))
    }

    pub fn collect_html_recovery_candidates(
        &self,
        context: &RequirementSourceManifestContext,
        entry: &Value,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementSourceRecoveryCandidates, String> {
        let adapter = select_requirement_source_adapter(entry);
        if adapter.parser != RequirementSourceParserKind::Html {
            return Ok(RequirementSourceRecoveryCandidates {
                generic_section_candidates: Vec::new(),
                parser_recovery_links: Vec::new(),
                supplemental_html_links: Vec::new(),
                supplemental_document_links: Vec::new(),
            });
        }
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
        Ok(RequirementSourceRecoveryCandidates {
            generic_section_candidates: context.generic_parser_recovery_section_candidates(
                entry,
                &artifacts.lines,
                artifacts.title.as_deref(),
                &artifacts.headings,
                course_parser,
            ),
            parser_recovery_links: context.extract_parser_recovery_link_candidates(
                entry,
                &artifacts.links,
                course_parser,
            ),
            supplemental_html_links: extract_supplemental_html_link_candidates(
                entry,
                &artifacts.links,
            ),
            supplemental_document_links: context.extract_supplemental_document_link_candidates(
                entry,
                &artifacts.links,
                course_parser,
            ),
        })
    }

    pub fn collect_source_facts(
        &self,
        entry: &Value,
        timeout: Duration,
        pdf_page_blocks: Option<&[PdfPageBlock]>,
        course_parser: &CourseCodeParser,
    ) -> Result<RequirementSourcePipelineResult, String> {
        let adapter = select_requirement_source_adapter(entry);
        let source_role = classify_requirement_source_role(entry);
        let url = entry.get("url").and_then(Value::as_str).unwrap_or_default();
        let parser_type = entry.get("parserType").and_then(Value::as_str);
        let facts = if is_pdf_source_url(url)
            || adapter.parser == RequirementSourceParserKind::Pdf
        {
            let page_blocks = pdf_page_blocks.ok_or_else(|| {
                format!("PDF source facts require extracted page blocks for {}", url)
            })?;
            collect_pdf_source_facts(entry, page_blocks, course_parser)
        } else if adapter.parser == RequirementSourceParserKind::Docx {
            let artifacts = self.docx_parser.get_docx_source_artifacts(url, timeout)?;
            collect_docx_source_facts(
                &artifacts,
                parser_type.unwrap_or("pdf-degree-sheet"),
                course_parser,
            )
        } else {
            let artifacts = self.html_parser.get_html_source_artifacts(url, timeout)?;
            collect_html_source_facts(&artifacts, parser_type.unwrap_or_default(), course_parser)
        };
        Ok(RequirementSourcePipelineResult {
            adapter,
            source_role,
            facts,
        })
    }
}

fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}
