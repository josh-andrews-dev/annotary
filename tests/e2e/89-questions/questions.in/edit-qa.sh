# Edit the pulled file's prose but leave the decisions table's Decided cells empty.
sed -i.bak 's/A new VM\./A new VM, bigger./' qa-plan.md && rm qa-plan.md.bak
