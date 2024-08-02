<script>
	import { onMount } from "svelte";
	import { writable } from "svelte/store";
	import Breadcrumb from "./Breadcrumb.svelte";
	import FileTable from "./FileTable.svelte";
	import { each } from "svelte/internal";

	const apiBaseUrl = "http://localhost:8090/admin";

	let currentPath = writable([]);
	let files = writable([]);
	let currentFolderFiles = writable([]);
	let sortBy = writable("name");
	let sortOrder = writable("asc");
	let filesToPublish = writable([]);

	async function fetchFiles() {
		const fullPath = `${apiBaseUrl}/list_files`;
		try {
			const response = await fetch(fullPath);
			if (!response.ok) {
				throw new Error("Failed to fetch files");
			}
			const data = await response.json();
			files.set(data);
			currentFolderFiles.set(data);
		} catch (error) {
			console.error("Error fetching files:", error);
			files.set([]);
		}
	}

	export async function createSharedLink() {
		const fullPath = `${apiBaseUrl}/create_shared_link`;
		try {
			const response = await fetch(fullPath, {
				method: "POST",
				headers: {
					"Content-Type": "application/json",
				},
				body: JSON.stringify({
					filesToPublish,
				}),
			});
			const data = await response.json();
			alert("Link created: " + data);
		} catch (error) {
			console.log(error);
		}
	}

	function updatePath(newPath) {
		console.log(newPath);
		currentPath.set(newPath);
		if (newPath.length === 0) {
			currentFolderFiles.set($files);
		} else {
			let tmp = $files;
			newPath.forEach((folder) => {
				tmp = tmp.filter((file) => file.name == folder)[0]["children"];
			});
			currentFolderFiles.set(tmp);
		}

		window.history.pushState({}, "", `/${newPath.join("/")}`);
	}

	// function filterSubPath(path, files) {
	// 	const filterByPath = files.filter((file) => file.name === path);
	// 	if filterByPath.length > 0 {

	// 		return filterByPath;
	// 	} else if (path === "") {
	// 		return files;
	// 	}
	// }

	function handleFolderClick(event) {
		const folderName = event.detail.folderName;
		currentPath.update((path) => {
			const newPath = [...path, folderName];
			window.history.pushState({}, "", `/${newPath.join("/")}`);

			//console.log(path);

			currentFolderFiles.set(
				$currentFolderFiles.filter(
					(file) => file.name === folderName,
				)[0]["children"],
			);
			return newPath;
		});
	}

	onMount(() => {
		const initialPath = window.location.pathname.split("/").filter(Boolean);
		updatePath(initialPath);
	});
	fetchFiles();

	$: currentPath, currentFolderFiles;
</script>

<div class="container">
	<Breadcrumb {currentPath} on:change={(e) => updatePath(e.detail)} />
	<FileTable
		files={currentFolderFiles}
		{sortBy}
		{sortOrder}
		on:sortChange={(e) => {
			sortBy.set(e.detail.by);
			sortOrder.set(e.detail.order);
		}}
		on:folderClick={handleFolderClick}
	/>
</div>

<style>
	/* Some basic styling */
	.container {
		padding: 1rem;
	}
</style>
