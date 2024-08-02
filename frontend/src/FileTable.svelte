<script>
    import { get } from "svelte/store";
    import { createEventDispatcher } from "svelte";

    export let files = [];
    export let selectedFiles = [];
    export let sortBy = "name";
    export let sortOrder = "asc";

    const dispatch = createEventDispatcher();

    // Function to pretty print file size
    function prettyPrintFileSize(sizeInBytes) {
        const units = ["Bytes", "KB", "MB", "GB", "TB"];
        let size = sizeInBytes;
        let unitIndex = 0;

        while (size >= 1024 && unitIndex < units.length - 1) {
            size /= 1024;
            unitIndex++;
        }

        return `${size.toFixed(2)} ${units[unitIndex]}`;
    }

    function sortFiles(by) {
        if (get(sortBy) === by) {
            sortOrder.set(get(sortOrder) === "asc" ? "desc" : "asc");
        } else {
            sortBy.set(by);
            sortOrder.set("asc");
        }

        const event = {
            detail: {
                by: get(sortBy),
                order: get(sortOrder),
            },
        };
        dispatch("sortChange", event);
    }

    function handleFolderClick(folderName) {
        console.log("Click folder");
        dispatch("folderClick", { folderName });
    }

    $: sortedFiles = [...$files].sort((a, b) => {
        const order = get(sortOrder) === "asc" ? 1 : -1;
        if (a[sortBy] < b[sortBy]) return -1 * order;
        if (a[sortBy] > b[sortBy]) return 1 * order;
        return 0;
    });
</script>

<button on:click={() => publishFiles(selectedFiles) }
<table>
    <thead>
        <tr>
            <th><input type="checkbox" disabled /> </th>
            <th on:click={() => sortFiles("name")}>Name</th>
            <th on:click={() => sortFiles("size")}>Size</th>
        </tr>
    </thead>
    <tbody>
        {#each sortedFiles as file}
            <tr>
                <td
                    ><input
                        type="checkbox"
                        bind:group={selectedFiles}
                        value={file.full_path}
                    />
                </td>
                <td
                    class={file.is_dir ? "folder" : "file"}
                    on:click={() => file.is_dir && handleFolderClick(file.name)}
                >
                    {file.is_dir ? "📁 " : "📄 "}
                    {file.name}
                </td>
                <td>{file.size ? prettyPrintFileSize(file.size) : "-"}</td>
            </tr>
        {/each}
    </tbody>
</table>

<style>
    table {
        width: 100%;
        border-collapse: collapse;
    }

    th,
    td {
        border: 1px solid #ccc;
        padding: 0.5rem;
        text-align: left;
    }
    td.folder {
        cursor: pointer;
        color: #333;
    }
    td.file {
        color: #333;
    }

    th {
        cursor: pointer;
        background-color: #f5f5f5;
    }
</style>
