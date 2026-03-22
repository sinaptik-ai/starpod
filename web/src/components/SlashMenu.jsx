import React, { useEffect, useRef } from 'react'

function SlashMenu({ skills, filter, activeIndex, onSelect, onHover }) {
  const listRef = useRef(null)

  const filtered = skills.filter(s =>
    s.name.toLowerCase().includes(filter.toLowerCase())
  )

  // Scroll active item into view
  useEffect(() => {
    if (!listRef.current) return
    const item = listRef.current.children[activeIndex]
    if (item) item.scrollIntoView({ block: 'nearest' })
  }, [activeIndex])

  if (filtered.length === 0) {
    return (
      <div className="slash-menu">
        <div className="slash-menu-empty">No skills found</div>
      </div>
    )
  }

  return (
    <div className="slash-menu" ref={listRef}>
      {filtered.map((skill, i) => (
        <div
          key={skill.name}
          className={`slash-menu-item${i === activeIndex ? ' active' : ''}`}
          onMouseDown={(e) => { e.preventDefault(); onSelect(skill) }}
          onMouseEnter={() => onHover(i)}
        >
          <span className="slash-menu-name">/{skill.name}</span>
          <span className="slash-menu-desc">{skill.description}</span>
        </div>
      ))}
    </div>
  )
}

export default SlashMenu
export function filterSkills(skills, filter) {
  return skills.filter(s =>
    s.name.toLowerCase().includes(filter.toLowerCase())
  )
}
