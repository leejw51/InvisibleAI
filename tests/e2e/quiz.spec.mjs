import { test, expect } from '@playwright/test';

test('random math has correct hidden answers and new numbers',async({page})=>{
  await page.goto('/quiz');
  await expect(page.getByRole('heading',{name:'Give your bot a little test.'})).toBeVisible();
  const question=page.getByTestId('quiz-question');
  const first=await question.textContent();
  await expect(page.getByTestId('quiz-answer')).toHaveCount(0);
  await page.getByRole('button',{name:'New numbers',exact:true}).click();
  await expect(question).not.toHaveText(first);
  const [a,op,b]=(await question.textContent()).split(' ');
  const expected=op==='+'? +a + +b : op==='−'?a-b:op==='×'?a*b:a/b;
  await page.getByRole('button',{name:'Show answer',exact:true}).click();
  await expect(page.getByTestId('quiz-answer').locator('pre')).toHaveText(String(expected));
  await page.getByRole('button',{name:'Capture this quiz',exact:true}).click();
  await expect(question).toBeVisible();
  await expect(page.getByTestId('quiz-answer')).toHaveCount(0);
  await expect(page.locator('.save-status')).toHaveText('Saved to your library');
  await page.screenshot({path:'test-results/quiz-math.png',fullPage:true});
});

test('reasoning and Rust coding quizzes randomize and fit a phone',async({page})=>{
  await page.goto('/quiz');
  const kinds=[['Sequence',null],['Word problem',null],['Quicksort','fn quicksort'],['Sliding window','fn max_window_sum'],['Binary search','fn binary_search'],['Two sum','fn two_sum'],['Predict the output','fn main'],['Producer / consumer','sync_channel']];
  for(const [label,code] of kinds){
    await page.getByRole('button',{name:label,exact:true}).click();
    const snippet=page.getByTestId('quiz-board').locator('code');
    if(code)await expect(snippet).toContainText(code);else await expect(snippet).toHaveCount(0);
    const first=await page.getByTestId('quiz-question').textContent();
    const firstCode=code?await snippet.textContent():'';
    await page.getByRole('button',{name:'New numbers',exact:true}).click();
    if(code==='fn main')await expect(snippet).not.toHaveText(firstCode);
    else await expect(page.getByTestId('quiz-question')).not.toHaveText(first);
    await page.getByRole('button',{name:'Show answer',exact:true}).click();
    await expect(page.getByTestId('quiz-answer')).toBeVisible();
  }
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  await page.screenshot({path:'test-results/quiz-rust-mobile.png',fullPage:true});
  await page.getByRole('button',{name:'Random quiz',exact:true}).click();
  await expect(page.getByTestId('quiz-answer')).toHaveCount(0);
});

// The page is an AI benchmark: every kind swaps its values on a 30-second clock,
// a click restarts the clock, and the checkbox or a running capture holds it.
test('every quiz changes its values every 30 seconds unless held',async({page})=>{
  // A paused clock makes the countdown deterministic; runFor fires each second's tick.
  await page.clock.install({time:new Date('2026-10-17T09:00:00')});
  await page.goto('/quiz');
  await expect(page.getByRole('heading',{name:'Give your bot a little test.'})).toBeVisible();
  await page.clock.pauseAt(new Date('2026-10-17T09:00:05'));
  const question=page.getByTestId('quiz-question');
  const countdown=page.getByTestId('quiz-countdown');
  await page.getByRole('button',{name:'Binary search',exact:true}).click();
  await expect(countdown).toContainText('Next quiz in 30s');
  const first=await question.textContent();
  await page.getByRole('button',{name:'Show answer',exact:true}).click();
  await page.clock.runFor(5_000);
  await expect(countdown).toContainText('Next quiz in 25s');
  await page.getByRole('button',{name:'New numbers',exact:true}).click();
  await expect(countdown).toContainText('Next quiz in 30s');
  const second=await question.textContent();
  expect(second).not.toBe(first);
  await page.clock.runFor(29_000);
  await expect(question).toHaveText(second);
  await expect(countdown).toContainText('Next quiz in 1s');
  await page.clock.runFor(1_000);
  await expect(question).not.toHaveText(second);
  await expect(question).toContainText('binary search');
  await expect(page.getByTestId('quiz-answer')).toHaveCount(0);
  await expect(countdown).toContainText('Next quiz in 30s');
  await page.getByRole('checkbox',{name:'Change every 30 s'}).uncheck();
  await expect(countdown).toContainText('Timer paused');
  const held=await question.textContent();
  await page.clock.runFor(45_000);
  await expect(question).toHaveText(held);
  await page.getByRole('checkbox',{name:'Change every 30 s'}).check();
  await page.getByRole('button',{name:'Math',exact:true}).click();
  const math=await question.textContent();
  await page.clock.runFor(30_000);
  await expect(question).not.toHaveText(math);
  expect(await question.textContent()).toMatch(/^\d+ [+−×÷] \d+ = \?$/);
});
