-- A PDF has no repository next to it, so a relative link such as
-- `../prices-api-load-test-100rps.md` points nowhere once rendered. Rewrite every
-- relative link to its GitHub URL on master; in-document anchors (`#…`), absolute
-- URLs and images are left alone.
local REPO = "https://github.com/rumblefishdev/stellar-prices-api/"
local BASE = "docs/scf/"   -- the directory the evidence documents live in

local function normalize(path)
  local parts = {}
  for seg in path:gmatch("[^/]+") do
    if seg == ".." then table.remove(parts)
    elseif seg ~= "." then table.insert(parts, seg) end
  end
  return table.concat(parts, "/")
end

function Link(el)
  local t = el.target
  if t:match("^%a[%w+.-]*:") or t:sub(1, 1) == "#" then return nil end
  local path, anchor = t:match("^([^#]*)(#?.*)$")
  local dir = path:sub(-1) == "/"
  el.target = REPO .. (dir and "tree" or "blob") .. "/master/" .. normalize(BASE .. path) .. (dir and "/" or "") .. anchor
  return el
end
