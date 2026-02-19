<script lang="ts">
	import '../app.css';
	import { base } from '$app/paths';
	import { page } from '$app/stores';
	import NotificationBar from '$lib/components/NotificationBar.svelte';
	import { clearToken } from '$lib/auth';
	import { goto } from '$app/navigation';

	let { children } = $props();

	const isAuthRoute = $derived(
		$page.url.pathname.startsWith(`${base}/auth/`) ||
			$page.url.pathname === base ||
			$page.url.pathname === `${base}/`,
	);

	function logout() {
		clearToken();
		goto(base || '/');
	}
</script>

{#if !isAuthRoute}
	<div class="min-h-screen bg-gray-950 text-white">
		<nav class="border-b border-gray-800 bg-gray-900 px-6 py-3 flex items-center justify-between">
			<div class="flex items-center gap-6">
				<span class="font-bold text-lg text-white">Hardwire</span>
				<a
					href="{base}/dashboard"
					class="text-gray-300 hover:text-white text-sm transition-colors"
				>
					Dashboard
				</a>
				<a href="{base}/files" class="text-gray-300 hover:text-white text-sm transition-colors">
					Files
				</a>
			</div>
			<div class="flex items-center gap-4">
				<NotificationBar />
				<button onclick={logout} class="text-gray-400 hover:text-white text-sm transition-colors">
					Logout
				</button>
			</div>
		</nav>
		<main class="p-6">
			{@render children()}
		</main>
	</div>
{:else}
	{@render children()}
{/if}
