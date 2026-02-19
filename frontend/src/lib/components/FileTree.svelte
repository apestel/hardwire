<script lang="ts">
	import FileTreeNode from './FileTreeNode.svelte';
	import type { FileInfo } from '$lib/types';

	let {
		nodes,
		selected,
		toggleSelect,
	}: {
		nodes: FileInfo[];
		selected: Set<string>;
		toggleSelect: (path: string) => void;
	} = $props();
</script>

<div class="font-mono text-xs text-gray-600 flex items-center gap-2 px-2 pb-1 border-b border-gray-800 mb-1 select-none">
	<span class="flex-1">Name</span>
	<span class="shrink-0 pl-4 hidden sm:block w-24 text-right">Created</span>
	<span class="shrink-0 pl-4 hidden sm:block w-24 text-right">Modified</span>
	<span class="shrink-0 pl-4 w-16 text-right">Size</span>
</div>
<ul class="space-y-0.5 font-mono text-sm">
	{#each nodes as node (node.full_path)}
		<FileTreeNode {node} {selected} {toggleSelect} depth={0} />
	{/each}
</ul>
