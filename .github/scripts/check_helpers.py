#!/usr/bin/env python3
import os
import sys
import re
import argparse

# Map of filename to expected (attr_count - name_count)
# Positive means more attributes than names (e.g. names omitting prefix)
# Negative means more names than attributes (e.g. missing attributes)
EXPECTED_OFFSETS = {
    "bitops.c": 3,  # 8 __rust_helper, 5 rust_helper_ (4 functions + 1 in comment)
}

def check_helpers(directory):
    files = [f for f in os.listdir(directory) if f.endswith('.c') and f != 'helpers.c']
    any_mismatch = False
    for filename in sorted(files):
        path = os.path.join(directory, filename)
        with open(path, 'r') as f:
            content = f.read()
            # Count occurrences of __rust_helper as a whole word
            rust_helper_attr_count = len(re.findall(r'\b__rust_helper\b', content))
            # Count occurrences of names starting with rust_helper_
            rust_helper_name_count = len(re.findall(r'\brust_helper_', content))
            
            actual_offset = rust_helper_attr_count - rust_helper_name_count
            expected_offset = EXPECTED_OFFSETS.get(filename, 0)
            
            if actual_offset != expected_offset:
                print(f"ERROR: {filename} has {rust_helper_attr_count} __rust_helper and {rust_helper_name_count} rust_helper_")
                print(f"       Expected offset {expected_offset}, got {actual_offset}")
                any_mismatch = True
    
    if not any_mismatch:
        print("All helpers have matching __rust_helper prefixes (accounting for known exceptions).")
    else:
        sys.exit(1)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description='Check for missing __rust_helper prefixes.')
    parser.add_argument('directory', help='Directory containing helper files')
    
    args = parser.parse_args()
    check_helpers(args.directory)
