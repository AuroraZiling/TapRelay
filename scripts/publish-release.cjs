const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');

function releaseAssets(directory, version) {
    const files = new Map();
    function read(name) {
        const data = fs.readFileSync(path.join(directory, name));
        if (!data.length) throw new Error(`Empty release asset: ${name}`);
        files.set(name, data);
        return data;
    }
    const prefix = `TapRelay_${version}_`;
    for (const suffix of ['Windows_x64_Setup.exe', 'Windows_x64.zip', 'macOS_arm64.pkg', 'macOS_x64.pkg']) {
        read(prefix + suffix);
    }
    for (const channel of ['win', 'osx-arm64', 'osx-x64']) {
        const feed = `releases.${channel}.json`;
        const { Assets: entries } = JSON.parse(read(feed));
        const full = entries?.[0];
        const suffix = channel === 'win' ? '' : `-${channel}`;
        if (entries?.length !== 1 || full?.Type !== 'Full' || full.PackageId !== 'TapRelay' ||
            full.Version !== version || full.FileName !== `TapRelay-${version}${suffix}-full.nupkg`) {
            throw new Error(`Expected one TapRelay ${version} full package in ${feed}.`);
        }
        const data = read(full.FileName);
        if (data.length !== full.Size || createHash('sha256').update(data).digest('hex') !== full.SHA256?.toLowerCase()) {
            throw new Error(`${full.FileName} does not match ${feed}.`);
        }
    }
    return files;
}

async function publish(github, context, tag, version, directory) {
    const repo = context.repo;
    const files = releaseAssets(directory, version);
    const { git, repos } = github.rest;
    // Never recreate a tag removed while the build was running.
    const { data: ref } = await git.getRef({ ...repo, ref: `tags/${tag}` });
    let object = ref.object;
    while (object.type === 'tag') {
        const { data } = await git.getTag({ ...repo, tag_sha: object.sha });
        object = data.object;
    }
    if (object.type !== 'commit' || object.sha !== context.sha) {
        throw new Error('The tag no longer points to the commit that was built.');
    }
    let release;
    try {
        ({ data: release } = await repos.getReleaseByTag({ ...repo, tag }));
    } catch (error) {
        if (error.status !== 404) throw error;
        // The tag endpoint only promises published releases. Resume a failed upload's draft.
        const releases = await github.paginate(repos.listReleases, { ...repo, per_page: 100 });
        release = releases.find(item => item.draft && item.tag_name === tag);
        if (!release) {
            ({ data: release } = await repos.createRelease({
                ...repo, tag_name: tag, name: `TapRelay ${tag}`,
                draft: true, prerelease: false, generate_release_notes: true,
            }));
        }
    }
    if (!release.draft) throw new Error(`${tag} is already published. Use a new version.`);
    const target = { ...repo, release_id: release.id };
    const existing = await github.paginate(repos.listReleaseAssets, { ...target, per_page: 100 });
    const unexpected = existing.find(asset => !files.has(asset.name));
    if (unexpected) throw new Error(`Unexpected draft asset: ${unexpected.name}. Review the draft before publishing.`);
    for (const [name, data] of files) {
        for (const asset of existing.filter(asset => asset.name === name)) {
            await repos.deleteReleaseAsset({ ...repo, asset_id: asset.id });
        }
        const type = name.endsWith('.json') ? 'application/json' : name.endsWith('.zip') ? 'application/zip' : 'application/octet-stream';
        await repos.uploadReleaseAsset({
            ...target, name, data, headers: { 'content-type': type, 'content-length': data.length },
        });
    }
    await repos.updateRelease({ ...target, draft: false });
}

module.exports = { publish, releaseAssets };
