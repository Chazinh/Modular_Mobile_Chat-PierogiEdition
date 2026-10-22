import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';

const chatViewUrl = new URL('../client/src/components/ChatView.jsx', import.meta.url);
const chatViewSource = fs.readFileSync(chatViewUrl, 'utf8');

const thresholdMatch = chatViewSource.match(/export const SCROLL_BOTTOM_THRESHOLD = (\d+);/);
const fnMatch = chatViewSource.match(/export function isAtBottom\([\s\S]*?\n\}/);

let isAtBottom;
if (fnMatch) {
  const mod = await import('data:text/javascript,' + encodeURIComponent(fnMatch[0]));
  isAtBottom = mod.isAtBottom;
}

export const SCROLL_BOTTOM_THRESHOLD = 16;

export function calculateAtEnd(scrollHeight, clientHeight, scrollTop, threshold = SCROLL_BOTTOM_THRESHOLD) {
  if (scrollHeight == null || clientHeight == null || scrollTop == null) return true;
  return (scrollHeight - clientHeight - scrollTop) <= threshold;
}

export function isScrollButtonVisible(stuck) {
  return !stuck;
}

export function createChatScrollModel(initial = {}) {
  let scrollHeight = initial.scrollHeight ?? 1000;
  let clientHeight = initial.clientHeight ?? 400;
  let scrollTop = initial.scrollTop ?? 600; // default at bottom: 1000 - 400 = 600
  let following = initial.following ?? true;
  let stuck = initial.stuck ?? true;
  let newContent = initial.newContent ?? false;

  return {
    getState() {
      return { scrollHeight, clientHeight, scrollTop, following, stuck, newContent };
    },
    isButtonVisible() {
      return isScrollButtonVisible(stuck);
    },
    onScroll(newTop) {
      scrollTop = newTop;
      const atEnd = calculateAtEnd(scrollHeight, clientHeight, scrollTop, SCROLL_BOTTOM_THRESHOLD);
      following = atEnd;
      stuck = atEnd;
      if (atEnd) {
        newContent = false;
      }
    },
    onNewContentArrived(addedHeight) {
      scrollHeight += addedHeight;
      newContent = true;
      if (following) {
        scrollTop = Math.max(0, scrollHeight - clientHeight);
        stuck = true;
        newContent = false;
      } else {
        stuck = false;
      }
    },
    toBottom() {
      following = true;
      stuck = true;
      newContent = false;
      scrollTop = Math.max(0, scrollHeight - clientHeight);
    }
  };
}

// -----------------------------------------------------------------------------
// Test Group 1: Metric Calculation Tests (Threshold <= 16 & Subpixel Resilience)
// -----------------------------------------------------------------------------

test('R4: SCROLL_BOTTOM_THRESHOLD is exported with value 16', () => {
  assert.ok(thresholdMatch, 'ChatView.jsx must export SCROLL_BOTTOM_THRESHOLD');
  assert.equal(Number(thresholdMatch[1]), 16, 'SCROLL_BOTTOM_THRESHOLD must equal 16px');
});

test('R4: isAtBottom calculation logic from ChatView.jsx', () => {
  assert.ok(isAtBottom, 'isAtBottom must be extracted and callable from ChatView.jsx');

  // Exactly at bottom: distance = 0
  assert.equal(isAtBottom(1000, 600, 400, 16), true, 'distance = 0 is at bottom');

  // Within bottom buffer (1 to 16px)
  assert.equal(isAtBottom(1000, 600, 395, 16), true, 'distance = 5 is at bottom');
  assert.equal(isAtBottom(1000, 600, 384, 16), true, 'distance = 16 is at bottom');

  // Fractional / subpixel positions
  assert.equal(isAtBottom(1000, 600, 384.4, 16), true, 'distance = 15.6 is at bottom (subpixel)');

  // Scrolled up beyond 16px threshold
  assert.equal(isAtBottom(1000, 600, 383, 16), false, 'distance = 17 is not at bottom');
  assert.equal(isAtBottom(1000, 600, 200, 16), false, 'distance = 200 is not at bottom');
  assert.equal(isAtBottom(1000, 600, 0, 16), false, 'scrollTop = 0 is not at bottom');

  // Short content without overflow (scrollHeight <= clientHeight)
  assert.equal(isAtBottom(500, 600, 0, 16), true, 'no overflow (scrollHeight < clientHeight) is at bottom');
  assert.equal(isAtBottom(600, 600, 0, 16), true, 'no overflow (scrollHeight == clientHeight) is at bottom');

  // Null/falsy fallback
  assert.equal(isAtBottom(null, 600, 0, 16), true, 'null metrics return true defensively');
});

test('R4: calculateAtEnd returns true when scroll offset is within threshold <= 16', () => {
  const scrollHeight = 1000;
  const clientHeight = 400;

  // Exact bottom: distance = 0
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 600), true, 'Exact bottom (0px) must be atEnd');

  // Subpixel offsets: distance = 0.5px, 1.25px, 1.5px, 5px, 15px
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 599.5), true, '0.5px from bottom must be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 598.75), true, '1.25px from bottom must be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 598.5), true, '1.5px from bottom must be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 595.0), true, '5.0px from bottom must be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 585.0), true, '15.0px from bottom must be atEnd');

  // Exact boundary threshold: distance = 16.0px
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 584.0), true, 'Exactly 16.0px from bottom must be atEnd');

  // Rubberbanding / momentum overscroll past bottom: distance < 0
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 620), true, 'Overscroll past bottom (-20px) must be atEnd');

  // Content fits entirely within viewport (no scrollable overflow)
  assert.equal(calculateAtEnd(300, 500, 0), true, 'Non-overflowing content must be atEnd');
  assert.equal(calculateAtEnd(500, 500, 0), true, 'Content exactly fitting viewport must be atEnd');
  assert.equal(calculateAtEnd(null, 500, 0), true, 'Defensive null check must be atEnd');
});

test('R4: calculateAtEnd returns false when scrolled up beyond 16px threshold', () => {
  const scrollHeight = 1000;
  const clientHeight = 400;

  // Just beyond threshold: 16.01px, 17px
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 583.99), false, '16.01px from bottom must NOT be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 583.0), false, '17.0px from bottom must NOT be atEnd');

  // Scrolled further up: 50px, 200px, 600px (top)
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 550), false, '50px from bottom must NOT be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 400), false, '200px from bottom must NOT be atEnd');
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, 0), false, 'Scrolled to top must NOT be atEnd');

  // Overscroll at top (pull-to-refresh bounce)
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, -30), false, 'Top overscroll bounce must NOT be atEnd');
});

test('R4: Subpixel sensitivity contrast - threshold <= 16 fixes false negative in obsolete <= 1', () => {
  const scrollHeight = 1000;
  const clientHeight = 400;
  const fractionalTop = 598.5; // distance = 1.5px
  const obsoleteAtEnd = (h, c, t) => h - c - t <= 1;

  // Obsolete <= 1 falsely evaluated to false on subpixel screens
  assert.equal(obsoleteAtEnd(scrollHeight, clientHeight, fractionalTop), false, 'Obsolete <= 1 failed on 1.5px subpixel distance');

  // Corrected <= 16 accurately detects bottom
  assert.equal(calculateAtEnd(scrollHeight, clientHeight, fractionalTop), true, 'Threshold <= 16 correctly recognizes 1.5px as atEnd');
});

// -----------------------------------------------------------------------------
// Test Group 2: Visibility Condition Verification (!stuck)
// -----------------------------------------------------------------------------

test('R4: Visibility condition - Button is strictly hidden when viewport is at bottom (stuck === true)', () => {
  assert.equal(isScrollButtonVisible(true), false, 'Button must be hidden when stuck=true');

  // Defect contrast: In buggy code (!stuck || newContent), newContent=true made button visible at bottom
  const buggyVisibility = (stuck, newContent) => !stuck || newContent;
  assert.equal(buggyVisibility(true, true), true, 'Demonstrates defect: old condition showed button at bottom');
  assert.equal(isScrollButtonVisible(true), false, 'Corrected condition: button remains hidden at bottom regardless of newContent');
});

test('R4: Visibility condition - Button is visible when scrolled up away from bottom (stuck === false)', () => {
  assert.equal(isScrollButtonVisible(false), true, 'Button must be visible when stuck=false');
});

// -----------------------------------------------------------------------------
// Test Group 3: State Transitions and User Actions
// -----------------------------------------------------------------------------

test('R4: State Machine - Clicking toBottom resets scroll, sets stuck=true, resets newContent, and hides button', () => {
  const model = createChatScrollModel({
    scrollHeight: 1000,
    clientHeight: 400,
    scrollTop: 200, // scrolled up
    following: false,
    stuck: false,
    newContent: true,
  });

  assert.equal(model.isButtonVisible(), true, 'Button must be visible before toBottom');

  // User taps floating button
  model.toBottom();

  const state = model.getState();
  assert.equal(state.scrollTop, 600, 'scrollTop must be reset to max bottom offset');
  assert.equal(state.stuck, true, 'stuck must be true');
  assert.equal(state.following, true, 'following must be true');
  assert.equal(state.newContent, false, 'newContent must be reset to false');
  assert.equal(model.isButtonVisible(), false, 'Button must be hidden after toBottom');
});

test('R4: State Machine - Manual scroll down to bottom automatically hides button', () => {
  const model = createChatScrollModel({
    scrollHeight: 1000,
    clientHeight: 400,
    scrollTop: 200,
    following: false,
    stuck: false,
  });

  assert.equal(model.isButtonVisible(), true, 'Button visible while reading up');

  // User scrolls down to within 16px of bottom (scrollTop = 590, distance = 10px)
  model.onScroll(590);

  const state = model.getState();
  assert.equal(state.stuck, true, 'stuck must be updated to true upon reaching bottom');
  assert.equal(state.following, true, 'following must be engaged');
  assert.equal(state.newContent, false, 'newContent must be cleared');
  assert.equal(model.isButtonVisible(), false, 'Button must automatically disappear on bottom reach');
});

test('R4: State Machine - Streaming tokens while following vs while scrolled up', () => {
  // Case A: User is following at bottom
  const followingModel = createChatScrollModel({
    scrollHeight: 1000,
    clientHeight: 400,
    scrollTop: 600,
    following: true,
    stuck: true,
  });

  // Token chunks arrive, growing content by 50px each
  followingModel.onNewContentArrived(50);
  assert.equal(followingModel.getState().scrollTop, 650);
  assert.equal(followingModel.isButtonVisible(), false, 'Button remains hidden during streaming at bottom');

  followingModel.onNewContentArrived(50);
  assert.equal(followingModel.getState().scrollTop, 700);
  assert.equal(followingModel.isButtonVisible(), false, 'Button remains hidden during streaming at bottom');

  // Case B: User scrolled up reading history
  const readingModel = createChatScrollModel({
    scrollHeight: 1000,
    clientHeight: 400,
    scrollTop: 200,
    following: false,
    stuck: false,
  });

  assert.equal(readingModel.isButtonVisible(), true);

  // Token chunks arrive while user is scrolled up
  readingModel.onNewContentArrived(50);
  assert.equal(readingModel.getState().scrollTop, 200, 'Scroll position preserved at reading anchor');
  assert.equal(readingModel.getState().stuck, false);
  assert.equal(readingModel.isButtonVisible(), true, 'Button remains visible for user to jump down');
});

test('R4: State Machine - Full continuous scrolling UX and jitter prevention', () => {
  let stuck = true;
  let following = true;
  let scrollingToBottom = false;
  let reRenderCount = 0;

  const setStuck = (val) => {
    if (stuck !== val) {
      stuck = val;
      reRenderCount++;
    }
  };

  const simulateScroll = (scrollHeight, clientHeight, scrollTop, prevTop) => {
    const atEnd = calculateAtEnd(scrollHeight, clientHeight, scrollTop);
    if (scrollingToBottom) {
      if (atEnd) {
        scrollingToBottom = false;
      } else if (prevTop != null && scrollTop < prevTop - 10) {
        scrollingToBottom = false;
        following = false;
        setStuck(false);
      }
    } else {
      if (following !== atEnd) {
        following = atEnd;
        setStuck(atEnd);
      }
    }
  };

  const simulateToBottom = () => {
    following = true;
    setStuck(true);
    scrollingToBottom = true;
  };

  // 1. Initially at bottom
  assert.equal(stuck, true);
  assert.equal(isScrollButtonVisible(stuck), false);
  assert.equal(reRenderCount, 0);

  // 2. User scrolls up: single re-render to mount button
  simulateScroll(1000, 600, 250, 400); // distance = 150 > 16
  assert.equal(following, false);
  assert.equal(stuck, false);
  assert.equal(isScrollButtonVisible(stuck), true);
  assert.equal(reRenderCount, 1);

  // 3. Continuous scroll up causes 0 extra re-renders
  simulateScroll(1000, 600, 150, 250);
  simulateScroll(1000, 600, 0, 150);
  assert.equal(reRenderCount, 1, 'Zero redundant re-renders during scroll-up');

  // 4. Scroll down to bottom threshold auto-hides button
  simulateScroll(1000, 600, 390, 0); // distance = 10 <= 16
  assert.equal(following, true);
  assert.equal(stuck, true);
  assert.equal(isScrollButtonVisible(stuck), false);
  assert.equal(reRenderCount, 2);

  // 5. Scroll up again and click toBottom
  simulateScroll(1000, 600, 200, 390);
  assert.equal(isScrollButtonVisible(stuck), true);
  assert.equal(reRenderCount, 3);

  simulateToBottom();
  assert.equal(stuck, true);
  assert.equal(isScrollButtonVisible(stuck), false);
  assert.equal(reRenderCount, 4);

  // Animation frames during smooth scroll do not cause button flicker
  simulateScroll(1000, 600, 300, 200);
  simulateScroll(1000, 600, 350, 300);
  assert.equal(stuck, true);
  assert.equal(reRenderCount, 4, 'Zero flicker re-renders during travel');

  simulateScroll(1000, 600, 400, 350);
  assert.equal(scrollingToBottom, false);
  assert.equal(stuck, true);
  assert.equal(reRenderCount, 4);
});

// -----------------------------------------------------------------------------
// Test Group 4: ChatView.jsx Source Code Structural Compliance
// -----------------------------------------------------------------------------

test('R4: ChatView.jsx source code conforms strictly to R4 scroll indicator requirements', () => {
  // 1. Threshold for atEnd must be <= 16 or SCROLL_BOTTOM_THRESHOLD = 16
  const hasThresholdConstant = chatViewSource.includes('SCROLL_BOTTOM_THRESHOLD = 16');
  const hasInlineThreshold = chatViewSource.includes('current.height - current.client - current.top <= 16') ||
                             chatViewSource.includes('scrollHeight - clientHeight - scrollTop <= 16');
  assert.ok(
    hasThresholdConstant || hasInlineThreshold,
    'ChatView.jsx must calculate atEnd with threshold <= 16 to absorb subpixel scroll fractions'
  );

  // 2. Obsolete fragile threshold <= 1 must not be used
  assert.ok(
    !chatViewSource.includes('current.height - current.client - current.top <= 1;'),
    'ChatView.jsx must NOT use obsolete fragile threshold <= 1'
  );

  // 3. Obsolete condition (!stuck || newContent) must not be used
  assert.ok(
    !chatViewSource.includes('(!stuck || newContent)'),
    'ChatView.jsx must NOT use (!stuck || newContent) which leaks button visibility at bottom'
  );

  // 4. Gating condition for scroll down button must be strictly !stuck
  assert.ok(
    chatViewSource.includes('{!stuck &&') ||
    chatViewSource.includes('{(!stuck) &&'),
    'ChatView.jsx must guard scroll indicator button strictly with !stuck'
  );

  // 5. Button attributes: onClick={toBottom} and <ArrowDown
  assert.ok(
    chatViewSource.includes('onClick={toBottom}'),
    'Scroll indicator button must bind onClick to toBottom'
  );
  assert.ok(
    chatViewSource.includes('<ArrowDown size={20} />') || chatViewSource.includes('<ArrowDown'),
    'Scroll indicator button must render ArrowDown icon'
  );

  // 6. toBottom handler resets stuck, followingRef, and restores scroll position
  assert.ok(
    chatViewSource.includes('followingRef.current = true;') &&
    chatViewSource.includes('setStuck(true);') &&
    chatViewSource.includes('restoreReading();'),
    'toBottom must set followingRef=true, setStuck(true), and call restoreReading()'
  );

  // 7. toBottom supports smooth scrolling
  assert.ok(
    chatViewSource.includes("behavior: 'smooth'"),
    'toBottom must support smooth scrolling behavior'
  );
});
