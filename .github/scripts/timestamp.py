import os
import subprocess
import sys

OUT_DIR = "../out"
HASH_FILE = os.path.join(OUT_DIR, "hashes.txt")

def main():
    # 1. Load stored hashes from the previous build (restored from cache)
    stored_map = {} # path -> hash
    if os.path.exists(HASH_FILE):
        try:
            with open(HASH_FILE, "r") as f:
                for line in f:
                    parts = line.strip().split(" ", 1)
                    if len(parts) == 2:
                        stored_map[parts[1]] = parts[0]
        except Exception as e:
            print(f"Warning: Could not read hashes.txt: {e}")

    # 2. Get current files from git to compare
    # We expect to be running inside the linux submodule directory
    cmd = ["git", "ls-tree", "-r", "HEAD"]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print("Error running git ls-tree")
        sys.exit(1)

    lines = result.stdout.splitlines()
    new_hashes = []
    
    # Timestamp for "unchanged" files: 2020-01-01 00:00:00 UTC
    # This ensures they are older than any build artifacts in 'out/'
    OLD_TIME = 1577836800
    
    matched_count = 0
    total_count = 0

    for line in lines:
        # git ls-tree output: <mode> <type> <hash>	<path>
        try:
            meta, path = line.split("\t", 1)
            meta_parts = meta.split()
            if len(meta_parts) < 3: continue
            obj_hash = meta_parts[2]
            
            # Save current state for the next build
            new_hashes.append(f"{obj_hash} {path}")
            total_count += 1
            
            # If the file content hasn't changed since the cached build,
            # set its timestamp to the past so 'make' considers the cached object valid.
            if path in stored_map and stored_map[path] == obj_hash:
                try:
                    os.utime(path, (OLD_TIME, OLD_TIME))
                    matched_count += 1
                except OSError:
                    pass
        except ValueError:
            continue

    print(f"Restored timestamps for {matched_count}/{total_count} files.")

    # 3. Write new hashes for the next build
    new_hashes.sort(key=lambda x: x.split(" ", 1)[1])
    try:
        with open(HASH_FILE, "w") as f:
            for item in new_hashes:
                f.write(item + "\n")
    except Exception as e:
        print(f"Error writing hashes.txt: {e}")

if __name__ == "__main__":
    main()
