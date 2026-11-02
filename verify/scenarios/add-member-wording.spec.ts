// A Member is a person or an agent, so the words that add one say so. On the real Members screen:
// the button, and a screenshot for the pull request (the top of the page holds only the owner's bar).
import { test } from '@playwright/test';
import { cli, expect, OUT } from '../lib';

test('the Members screen and the CLI say Member, not agent, when adding one', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('/settings/server/members');
  const button = page.locator('#cd-addmember');
  await expect(button).toHaveText('Add member');
  await expect(page.getByText('Add agent')).toHaveCount(0);
  await button.screenshot({ path: `${OUT}/add-member-button.png` });

  const help = cli(['member', '--help']);
  expect(help).toContain('a person or an agent');
  expect(help).not.toContain('how you add an agent');
});
