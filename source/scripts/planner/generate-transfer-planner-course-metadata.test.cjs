const test = require("node:test");
const assert = require("node:assert/strict");
const {
  extractTitleFromRow,
  updateGreenRiverArtScheduleTitles,
} = require("./generate-transfer-planner-course-metadata.cjs");

function makeRow(title, timeItems = []) {
  return {
    items: [
      { str: title, x: 100 },
      ...timeItems.map((str, index) => ({ str, x: 210 + index * 10 })),
    ],
  };
}

test("schedule title extraction excludes a trailing start time from the course name", () => {
  assert.equal(extractTitleFromRow(makeRow("Beginning Design", ["10:00A"])), "Beginning Design");
  assert.equal(extractTitleFromRow(makeRow("Painting 1", ["9:00A"])), "Painting 1");
  assert.equal(extractTitleFromRow(makeRow("Pottery 1", ["9:30", "A"])), "Pottery 1");
  assert.equal(extractTitleFromRow(makeRow("Studio Art", ["9:00 A.M. - 10:30 A.M."])), "Studio Art");
});

test("schedule title extraction preserves course numbers and titles without times", () => {
  assert.equal(extractTitleFromRow(makeRow("Advanced Drawing")), "Advanced Drawing");
  assert.equal(
    extractTitleFromRow(makeRow("Intermediate Design and Color")),
    "Intermediate Design and Color"
  );
  assert.equal(extractTitleFromRow(makeRow("Painting 2")), "Painting 2");
});

test("targeted Green River ART title refresh preserves all non-title metadata and other courses", () => {
  const existingEntries = [
    { schoolId: "grc", code: "ART 109", title: "Beginning Design 10:00A", notes: ["catalog note"] },
    { schoolId: "grc", code: "BIOL 110", title: "Old biology title" },
    { schoolId: "uw-seattle", code: "ART 109", title: "UW course title" },
  ];
  const scheduleEntries = [
    { schoolId: "grc", code: "ART 109", title: "Beginning Design" },
    { schoolId: "grc", code: "BIOL 110", title: "New biology title" },
  ];

  const result = updateGreenRiverArtScheduleTitles(existingEntries, scheduleEntries);

  assert.equal(result.updatedCount, 1);
  assert.deepEqual(result.entries, [
    { schoolId: "grc", code: "ART 109", title: "Beginning Design", notes: ["catalog note"] },
    { schoolId: "grc", code: "BIOL 110", title: "Old biology title" },
    { schoolId: "uw-seattle", code: "ART 109", title: "UW course title" },
  ]);
});
