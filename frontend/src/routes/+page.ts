import { browser } from '$app/environment';
import { redirect } from '@sveltejs/kit';
import { isAuthenticated } from '$lib/auth';
import { base } from '$app/paths';

export const prerender = false;
export const ssr = false;

export function load() {
	if (browser && isAuthenticated()) {
		redirect(302, `${base}/dashboard`);
	}
	return {};
}
