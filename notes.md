# From Purely top-down software rebuilding by Alan Grosskurth
## Algorithm 1 redo, part 1: redo procedure
```
procedure redo(args)
  delete *.uptodate
  redo-ifchange it
end procedure
```

## Algorithm 2 redo, part 2: redo-ifchange procedure
```
procedure redo-ifchange(args)
  create .redo database if it does not already exist
  for each argument i do
    if i.type does not exist then
      if i exists then
	i.type ← source
      else
	i.type ← target
      end if
    end if
    if i.uptodate exists then
      record i as a regular prerequiste for its parent
    continue
  end if
```

## Algorithm 3 redo, part 3: continuation of redo-ifchange procedure
```
  if i.type = source then
    if i.md5 exists and it matches the current MD5 hash then
      i.uptodate ← yes
    else
      i.uptodate ← no
      if i.nonexist exists then
	delete i.nonexist
      end if
    end if
    i.md5 ← current MD5 hash
    record i as a regular prerequiste for its parent
    continue
  end if
```

## Algorithm 4 redo, part 4: continuation of redo-ifchange procedure
```
  if i.prereqs exists then
    i.uptodate ← yes
    for each file j in i.prereqs do
      if j.uptodate does not exist then
	redo-ifchange j
	if j.uptodate = no then
	  i.uptodate ← no end if
	end if
      end for
    end if
    if i.prereqsnonexist exists then
      for each file j in i.prereqsnonexist do
	if j exists then
	  i.uptodate ← no
	end if
      end for end if
    if i.uptodate = yes then
      break
    end if
```

## Algorithm 5 redo, part 5: continuation of redo-ifchange procedure
```
    if build file i.do exists then
      redo-ifchange i.do
      i.buildfile ← i.do
    else
      calculate filename for default build script if
      if default build script exists then
	redo-ifchange default
	redo-ifcreate i.do
	i.buildfile ← default
      else
	error: no build script for i
      end if
    end if
```

## Algorithm 6 redo, part 6: continuation of redo-ifchange procedure
```
    execute the build script for i and store the result
    if result ̸!= 0 then
      error: build failed
    end if
    if i.md5 exists and matches the current MD5 hash then
      record i.uptodate = yes
    end if
    record i.md5 = current MD5 hash
  end for
  record i as a regular prerequiste for its parent
end procedure
```

## Algorithm 7 redo, part 7: redo-ifcreate procedure
```
procedure redo-ifcreate(args)
  for each argument i do
    if i exists then
      error
    end if
    record i as a nonexistent prerequiste for its parent
    delete i.md5
    create i.nonexist
  end for
end procedure
```
