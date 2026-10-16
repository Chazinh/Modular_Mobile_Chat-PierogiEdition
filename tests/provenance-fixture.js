import fs from 'node:fs';
export const mira = JSON.parse(fs.readFileSync(new URL('./fixtures/quote-attribution.json', import.meta.url), 'utf8'));
export const [previousBot, currentUser, falseBot] = mira.messages.map(m => m.content);
export const noFlags = () => ({ agency: [], knowledge: [], continuity: [], attribution: [] });
export const attributionReview = () => ({ ...noFlags(), attribution: [structuredClone(mira.attributionFlag)] });
export const extractionFixture = () => ({
  summary: mira.summary, cast: [],
  scene: { location: 'corredor', time: '', mood: '', present: ['Alex', 'Mira'], objects: [], characters: [] },
  facts: ['Alex', 'Mira'].map((originalSpeaker, i) => ({
    kind: 'EVENT', subject: originalSpeaker, content: i ? mira.correctFact : mira.wrongFact,
    importance: .5, confidence: 1, visibility: 'private', replaces: '', existing_id: '',
    attribution: { quote: mira.quote, originalSpeaker },
    knowledge: [{ who: 'Mira', state: 'knows', via: 'heard', evidence: mira.quote, belief: '' }],
  })),
  relationships: [], open_threads: [], resolved_threads: [],
});
// Contaminated batch: the summary ITSELF carries the false accusation, along with a fact with inverted authorship.
export const contaminatedSummary = 'Alex chamou Mira de carteira das ilhas.';
export const contaminatedExtraction = () => ({ ...extractionFixture(), summary: contaminatedSummary });
// Valid batch without inversion: only Mira's fact, with the documented authorship.
export const validExtraction = () => {
  const e = extractionFixture();
  e.facts = e.facts.slice(1);
  return e;
};
// The same valid fact WITHOUT attribution metadata (legacy/optional schema): never triggers the block.
export const legacyExtraction = () => {
  const e = validExtraction();
  delete e.facts[0].attribution;
  return e;
};

// Contract of the authorship-review schema fragment shared between the Node tests and provenance_contracts.rs
// (fixtures/review-attribution-schema.json): `required` sorted so the comparison does not depend on order.
export const reviewAttributionContract = JSON.parse(fs.readFileSync(new URL('./fixtures/review-attribution-schema.json', import.meta.url), 'utf8'));
export const normalizeReviewSchema = schema => ({
  topLevelRequired: [...schema.required].sort(),
  attribution: { ...schema.properties.attribution, items: { ...schema.properties.attribution.items, required: [...schema.properties.attribution.items.required].sort() } },
});
