<script lang="ts">
	import type { Notification } from '$lib/stores/notifications';

	let { notif, ondismiss }: { notif: Notification; ondismiss: () => void } = $props();

	const bg: Record<string, string> = {
		info: 'bg-blue-900 border-blue-700',
		success: 'bg-green-900 border-green-700',
		error: 'bg-red-900 border-red-700',
		progress: 'bg-gray-800 border-gray-600',
	};
</script>

<div class="rounded-lg border p-3 shadow-lg {bg[notif.kind]}">
	<div class="flex items-start justify-between gap-2">
		<span class="text-sm text-white leading-snug">{notif.message}</span>
		{#if notif.dismissible}
			<button
				onclick={ondismiss}
				class="text-gray-400 hover:text-white shrink-0 text-xs leading-none mt-0.5"
				aria-label="Dismiss"
			>
				✕
			</button>
		{/if}
	</div>
	{#if notif.kind === 'progress' && notif.progress !== undefined}
		<div class="mt-2 h-1.5 w-full bg-gray-700 rounded-full overflow-hidden">
			<div
				class="h-full bg-blue-500 rounded-full transition-all duration-300"
				style="width: {notif.progress}%"
			></div>
		</div>
	{/if}
</div>
