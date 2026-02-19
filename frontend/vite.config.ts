import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
	server: {
		proxy: {
			'/admin/api': 'http://localhost:8080',
			'/admin/auth': 'http://localhost:8080',
			'/admin/live_update': {
				target: 'ws://localhost:8080',
				ws: true,
			},
		},
	},
});
