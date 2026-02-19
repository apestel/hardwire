import adapter from '@sveltejs/adapter-static';

/** @type {import('@sveltejs/kit').Config} */
const config = {
	kit: {
		adapter: adapter({
			pages: '../dist/admin',
			assets: '../dist/admin',
			fallback: 'index.html',
		}),
		paths: {
			base: '/admin',
		},
	},
};

export default config;
