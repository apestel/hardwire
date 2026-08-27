<script lang="ts">
	import '../app.css';
	import { base } from '$app/paths';
	import { page, navigating } from '$app/stores';
	import NotificationBar from '$lib/components/NotificationBar.svelte';
	import { clearToken, verifyToken } from '$lib/auth';
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';

	let { children } = $props();

	const isAuthRoute = $derived(
		$page.url.pathname.startsWith(`${base}/auth/`) ||
			$page.url.pathname === base ||
			$page.url.pathname === `${base}/`,
	);

	// The client-side auth check trusts the (unverifiable) `exp` claim of the
	// JWT, so poll the backend: if it rejects the token (revoked, rotated
	// secret, deleted user), force a logout.
	onMount(() => {
		const check = async () => {
			const pathname = $page.url.pathname;
			const onAuthRoute =
				pathname.startsWith(`${base}/auth/`) ||
				pathname === base ||
				pathname === `${base}/`;
			if (onAuthRoute) return;
			if (!(await verifyToken())) {
				clearToken();
				goto(base || '/');
			}
		};
		void check();
		const timer = setInterval(() => void check(), 60_000);
		return () => clearInterval(timer);
	});

	function isActive(path: string): boolean {
		return $page.url.pathname.startsWith(`${base}${path}`);
	}

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
					class="text-sm transition-colors {isActive('/dashboard')
						? 'text-white font-medium border-b-2 border-blue-500 pb-0.5'
						: 'text-gray-400 hover:text-white'}"
				>
					Dashboard
				</a>
				<a
					href="{base}/files"
					class="text-sm transition-colors {isActive('/files')
						? 'text-white font-medium border-b-2 border-blue-500 pb-0.5'
						: 'text-gray-400 hover:text-white'}"
				>
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
		{#if $navigating}
			<div class="flex items-center justify-center py-20">
				<svg class="size-8 animate-spin text-blue-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
					<circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
					<path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
				</svg>
			</div>
		{:else}
			<main class="p-6">
				{@render children()}
			</main>
		{/if}
	</div>
{:else}
	{@render children()}
{/if}
