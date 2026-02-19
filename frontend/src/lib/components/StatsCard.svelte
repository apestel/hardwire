<script lang="ts">
	let { label, value }: { label: string; value: string } = $props();
</script>

<div class="bg-gray-900 border border-gray-800 rounded-xl p-5">
	<p class="text-gray-400 text-sm">{label}</p>
	<p class="text-white text-2xl font-bold mt-1">{value}</p>
</div>
