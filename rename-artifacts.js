import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const bundleDir = path.resolve(__dirname, 'src-tauri/target/release/bundle');

function renameInDir(dir) {
    if (!fs.existsSync(dir)) return;

    const files = fs.readdirSync(dir);
    for (const file of files) {
        const fullPath = path.join(dir, file);
        let stat;
        try {
            stat = fs.statSync(fullPath);
        } catch (e) {
            console.warn(`Skipping ${fullPath}: ${e.message}`);
            continue;
        }

        if (stat.isDirectory()) {
            renameInDir(fullPath);
        } else {
            if (file.includes(' ')) {
                const newName = file.replace(/ /g, '-');
                const newPath = path.join(dir, newName);
                console.log(`Renaming: ${file} -> ${newName}`);
                try {
                    fs.renameSync(fullPath, newPath);
                } catch (e) {
                    console.error(`Failed to rename ${file}: ${e.message}`);
                }
            }
        }
    }
}

console.log('Scanning for artifacts with spaces in filenames...');
renameInDir(bundleDir);
console.log('Artifact renaming complete.');
