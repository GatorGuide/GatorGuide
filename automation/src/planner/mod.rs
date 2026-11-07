mod grc_catalog;
mod grc_public_materials;

pub use grc_catalog::{
    GrcCourseCatalogInput, GrcCourseCatalogSection, GrcCourseDetails, GrcCourseDetailsInput,
    parse_grc_course_catalog, parse_grc_course_details,
};

use std::path::Path;

pub use grc_public_materials::{
    GrcAnnualScheduleCandidate, GrcAnnualScheduleCandidatesInput, GrcAnnualScheduleFilterInput,
    GrcCatalogArchiveEntriesInput, GrcCatalogEntry, GrcCurrentCatalog,
    GrcCurrentCatalogDetailsInput, GrcPublicMaterials, GrcSchedule,
    build_paged_course_descriptions_url, detect_catalog_page_numbers,
    filter_relevant_annual_schedule_values, parse_annual_schedule_candidates,
    parse_catalog_archive_entries, parse_current_catalog_details,
};

pub fn discover_grc_public_materials(
    repository_root: &Path,
    force_refresh: bool,
    allow_snapshot_fallback: bool,
    cache_only: bool,
) -> Result<GrcPublicMaterials, String> {
    grc_public_materials::discover(
        repository_root,
        force_refresh,
        allow_snapshot_fallback,
        cache_only,
    )
}
