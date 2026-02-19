<script lang="ts">
	let { data } = $props();
</script>

<div class="min-h-screen flex items-center justify-center bg-gray-950">
	{#if data.error}
		<div class="text-center">
			<p class="text-red-400 mb-4">Authentication failed</p>
			<p class="text-gray-500 text-sm">{data.error}</p>
			<a href="/admin/" class="mt-4 inline-block text-blue-400 hover:text-blue-300 text-sm">
				Back to login
			</a>
		</div>
	{:else}
		<div class="text-gray-400 animate-pulse">Authenticating...</div>
	{/if}
</div>
