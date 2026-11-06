const assert = require("node:assert/strict");
const test = require("node:test");

require("../../../scripts/qa/react-native-test-utils.cjs");

const { RESOURCE_CATALOG } = require("@/constants/resource-catalog");
const {
  countResourceSectionItems,
  filterVisibleResourceCatalogItems,
  isTransferPlannerResourceUrl,
} = require("@/components/pages/resources/useResourcesPageModel");

function getCatalogItems() {
  return RESOURCE_CATALOG.flatMap((section) => [
    ...(section.items ?? []),
    ...(section.subsections ?? []).flatMap((subsection) => subsection.items),
  ]);
}

test("Transfer Planner remains in the catalog but is excluded from visible resources and counts", () => {
  const catalogItems = getCatalogItems();
  const plannerItems = catalogItems.filter((item) => isTransferPlannerResourceUrl(item.url));

  assert.equal(plannerItems.length, 1);
  assert.equal(plannerItems[0].titleKey, "resources.transferPlanner");
  assert.equal(plannerItems[0].descriptionKey, "resources.transferPlannerDesc");

  const visibleCatalogItems = filterVisibleResourceCatalogItems(catalogItems);
  assert.equal(visibleCatalogItems.some((item) => isTransferPlannerResourceUrl(item.url)), false);

  const searchResults = visibleCatalogItems.filter((item) =>
    [item.title, item.titleKey, item.description, item.descriptionKey, ...(item.tags ?? [])]
      .join(" ")
      .toLowerCase()
      .includes("transfer planner")
  );
  assert.deepEqual(searchResults, []);

  const catalogCount = catalogItems.length;
  const visibleSections = RESOURCE_CATALOG.map((section) => ({
    items: filterVisibleResourceCatalogItems(section.items ?? []),
    subsections: (section.subsections ?? [])
      .map((subsection) => ({
        items: filterVisibleResourceCatalogItems(subsection.items),
      }))
      .filter((subsection) => subsection.items.length > 0),
  })).filter((section) => countResourceSectionItems(section) > 0);
  const visibleCount = visibleSections.reduce(
    (sum, section) => sum + countResourceSectionItems(section),
    0
  );

  assert.equal(visibleCount, catalogCount - plannerItems.length);
});
