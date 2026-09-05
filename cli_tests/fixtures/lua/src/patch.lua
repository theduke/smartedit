local M = {}

-- TODO: remove after the release.
function M.status()
  return "pending"
end

function M.add(left, right)
  return left + right
end

return M
