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

test('Rust coding quizzes randomize and fit a phone',async({page})=>{
  await page.goto('/quiz');
  for(const [label,code] of [['Quicksort','fn quicksort'],['Sliding window','fn max_window_sum'],['Producer / consumer','sync_channel']]){
    await page.getByRole('button',{name:label,exact:true}).click();
    await expect(page.getByTestId('quiz-board').locator('code')).toContainText(code);
    const first=await page.getByTestId('quiz-question').textContent();
    await page.getByRole('button',{name:'New numbers',exact:true}).click();
    await expect(page.getByTestId('quiz-question')).not.toHaveText(first);
    await page.getByRole('button',{name:'Show answer',exact:true}).click();
    await expect(page.getByTestId('quiz-answer')).toBeVisible();
  }
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  await page.screenshot({path:'test-results/quiz-rust-mobile.png',fullPage:true});
  await page.getByRole('button',{name:'Random quiz',exact:true}).click();
  await expect(page.getByTestId('quiz-answer')).toHaveCount(0);
});
