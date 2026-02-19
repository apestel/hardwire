<script lang="ts">
	import type { DownloadRecord } from '$lib/types';

	let { rows }: { rows: DownloadRecord[] } = $props();

	function formatTs(ts: number) {
		return new Date(ts * 1000).toLocaleString();
	}

	function formatSize(bytes: number | null) {
		if (!bytes) return '—';
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
		return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
	}
</script>

{#if rows.length === 0}
	<p class="text-gray-500 text-sm text-center py-8">No downloads recorded yet.</p>
{:else}
	<div class="overflow-x-auto">
		<table class="w-full text-sm text-left">
			<thead>
				<tr class="border-b border-gray-800 text-gray-400">
					<th class="pb-2 pr-4 font-medium">File</th>
					<th class="pb-2 pr-4 font-medium">IP</th>
					<th class="pb-2 pr-4 font-medium">Status</th>
					<th class="pb-2 pr-4 font-medium">Size</th>
					<th class="pb-2 pr-4 font-medium">Started</th>
					<th class="pb-2 font-medium">Finished</th>
				</tr>
			</thead>
			<tbody>
				{#each rows as row (row.id)}
					<tr class="border-b border-gray-800/50 hover:bg-gray-800/30">
						<td class="py-2 pr-4 text-gray-200 truncate max-w-xs" title={row.file_path}>
							{row.file_path.split('/').pop() ?? row.file_path}
						</td>
						<td class="py-2 pr-4 text-gray-400 font-mono text-xs">{row.ip_address || '—'}</td>
						<td class="py-2 pr-4">
							<span
								class="px-2 py-0.5 rounded-full text-xs font-medium
								{row.status === 'complete'
									? 'bg-green-900 text-green-300'
									: row.status === 'in_progress'
									? 'bg-blue-900 text-blue-300'
									: 'bg-gray-700 text-gray-300'}"
							>
								{row.status}
							</span>
						</td>
						<td class="py-2 pr-4 text-gray-400">{formatSize(row.file_size)}</td>
						<td class="py-2 pr-4 text-gray-400 whitespace-nowrap">{formatTs(row.started_at)}</td>
						<td class="py-2 text-gray-400 whitespace-nowrap">{row.finished_at ? formatTs(row.finished_at) : '—'}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
