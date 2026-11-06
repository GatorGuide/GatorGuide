const assert = require("node:assert/strict");
const test = require("node:test");
const { HttpsError } = require("firebase-functions/v2/https");
const { __test } = require("./geminiGateway");

function buildConfig(overrides = {}) {
  return {
    authDailyUnitsLimit: 8,
    globalDailyUnitsLimit: 10,
    guestDailyUnitsLimit: 5,
    recommendBatchSize: 2,
    maxCandidatesPerRequest: 4,
    ...overrides,
  };
}

function buildIdentity(overrides = {}) {
  return {
    authUid: null,
    clientInstanceId: "client-1",
    clientKeyHash: "client-hash",
    scope: "guest",
    ...overrides,
  };
}

test("Gemini prompt inputs are bounded before template rendering", () => {
  const colleges = Array.from({ length: 10 }, (_, index) => ({
    id: `college-${index}`,
    name: `College ${index} ${"x".repeat(300)}`,
    location: { city: "c".repeat(200), state: "Washington".repeat(20) },
    reason: "good fit ".repeat(80),
    programs: Array.from({ length: 20 }, (__, programIndex) =>
      `Program ${programIndex} ${"p".repeat(220)}`
    ),
  }));

  const { input, promptTemplate } = __test.buildChatAssistantPrompt({
    query: "q".repeat(__test.MAX_CHAT_MESSAGE_CHARS + 200),
    context: { notes: "n".repeat(20000) },
    outputFormat: "please-ignore-json",
    topRankedColleges: colleges,
  });

  assert.equal(input.query.length, __test.MAX_CHAT_MESSAGE_CHARS);
  assert.equal(input.outputFormat, "text");
  assert.equal(input.topRankedColleges.length, __test.MAX_ASSISTANT_COLLEGES);
  assert.equal(input.topRankedColleges[0].name.length, 160);
  assert.equal(input.topRankedColleges[0].reason.length, 220);
  assert.equal(input.topRankedColleges[0].programs.length, __test.MAX_PROGRAMS_PER_COLLEGE);
  assert.ok(input.topRankedColleges[0].programs.every((program) => program.length <= 120));
  assert.match(promptTemplate.prompt, /Prompt template: chat-assistant@/);
  assert.doesNotMatch(promptTemplate.prompt, /college-7/);
});

test("Gemini sanitizers reject oversized documents and candidate batches", () => {
  assert.throws(
    () =>
      __test.sanitizeDocumentExtractionInput({
        documentType: "transcript",
        mimeType: "application/pdf",
        fileBase64: "x".repeat(__test.MAX_DOCUMENT_BASE64_CHARS + 1),
      }),
    (error) => error instanceof HttpsError && error.code === "invalid-argument"
  );

  assert.throws(
    () =>
      __test.sanitizeRecommendInput(
        {
          colleges: Array.from({ length: 5 }, (_, index) => ({
            id: `college-${index}`,
            name: "College",
          })),
        },
        buildConfig({ maxCandidatesPerRequest: 4 })
      ),
    (error) => error instanceof HttpsError && error.code === "invalid-argument"
  );
});

test("Gemini document extraction responses keep only supported bounded fields", () => {
  const response = __test.sanitizeDocumentExtractionResponse(
    {
      documentType: "transcript-and-resume".repeat(4),
      extractedFields: {
        completedCourses: {
          value: Array.from({ length: 25 }, (_, index) => `COURSE ${index} ${"x".repeat(150)}`),
          sourceSnippet: "s".repeat(400),
          confidence: 120,
        },
        unsupported: {
          value: "should be dropped",
        },
      },
      uncertainties: Array.from({ length: 12 }, (_, index) => `uncertain ${index} ${"u".repeat(400)}`),
      confidence: 999,
    },
    "transcript"
  );

  assert.equal(response.documentType.length, 40);
  assert.equal(response.extractedFields.completedCourses.value.length, 20);
  assert.ok(
    response.extractedFields.completedCourses.value.every((item) => item.length <= 120)
  );
  assert.equal(response.extractedFields.completedCourses.sourceSnippet.length, 240);
  assert.equal(response.extractedFields.completedCourses.confidence, 100);
  assert.equal(response.uncertainties.length, 8);
  assert.equal(response.confidence, 100);
  assert.equal(response.extractedFields.unsupported, undefined);
});

test("Gemini in-memory quota reservation accounts for attempts and finalized usage", async () => {
  const store = new __test.InMemoryUsageStore();
  const identity = buildIdentity();
  const reservation = await __test.reserveUsage(
    buildConfig(),
    "recommendFactors",
    2,
    identity,
    store
  );

  assert.equal(reservation.allowed, true);
  assert.equal(reservation.units, 2);
  assert.equal(reservation.clientRemaining, 3);
  assert.equal(reservation.globalRemaining, 8);

  await __test.finalizeUsage(
    reservation,
    {
      status: "success",
      usage: { promptTokens: 4, candidateTokens: 6, totalTokens: 10 },
      responseMs: 123,
      model: "gemini-test",
      promptTemplate: {
        id: "recommend-factor-scoring",
        version: "v1",
        libraryVersion: "test-lib",
      },
    },
    store
  );

  const day = store.days.get(reservation.dateKey);
  const client = day.clients.get(identity.clientKeyHash);
  assert.equal(day.global.requests, 1);
  assert.equal(day.global.units, 2);
  assert.equal(day.global.successes, 1);
  assert.equal(day.global.totalTokens, 10);
  assert.equal(day.global.actions.recommendFactors.promptTokens, 4);
  assert.equal(client.lastModel, "gemini-test");
  assert.equal(client.lastPromptTemplateId, "recommend-factor-scoring");
});

test("Gemini in-memory quotas record and deny over-limit attempts", async () => {
  const store = new __test.InMemoryUsageStore();
  const identity = buildIdentity();
  const config = buildConfig({ globalDailyUnitsLimit: 10, guestDailyUnitsLimit: 10 });
  const allowed = await __test.reserveUsage(config, "chatAssistant", 9, identity, store);
  const denied = await __test.reserveUsage(config, "chatAssistant", 2, identity, store);

  assert.equal(allowed.allowed, true);
  assert.equal(denied.allowed, false);
  assert.equal(denied.reason, "global_limit");
  const day = store.days.get(allowed.dateKey);
  assert.equal(day.global.units, 9);
  assert.equal(day.global.quotaDenied, 1);
  assert.equal(day.global.actions.chatAssistant.quotaDenied, 1);
});

test("Gemini action parsing and upstream errors map to callable error codes", () => {
  assert.deepEqual(
    __test.parseAction(
      { action: "recommendFactors", colleges: [{ id: "a" }, { id: "b" }, { id: "c" }] },
      buildConfig({ recommendBatchSize: 2 })
    ),
    { action: "recommendFactors", units: 2 }
  );

  assert.equal(__test.parseGeminiError(429, "RESOURCE_EXHAUSTED quota").code, "resource-exhausted");
  assert.equal(__test.parseGeminiError(408, "deadline exceeded").code, "deadline-exceeded");
  assert.equal(__test.parseGeminiError(503, "service down").code, "unavailable");
  assert.equal(__test.parseGeminiError(400, "bad request").code, "internal");
});
