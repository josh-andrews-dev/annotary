// The CLI's own words are American English: the global flag's help, on a verb and at the top.
import { test } from '@playwright/test';
import { cli, expect } from '../lib';

test('the help says color', () => {
  for (const args of [['--help'], ['list', '--help']]) {
    expect(cli(args)).not.toMatch(/colour/i);
  }
  expect(cli(['list', '--help'])).toMatch(/no color/i);
});
